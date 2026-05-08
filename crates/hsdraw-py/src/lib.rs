//! `hsdraw` Python module — PyO3 binding.
//!
//! Surface (matches `docs/python_api.md` and the table in the project
//! handoff): every Python class is a thin wrapper around an Rc-shared
//! `hsdraw_core` value, so identity (Rc::ptr_eq) is preserved across
//! Python operations and `dat.find_root_for(jobj)` works without
//! re-walking the tree.
//!
//! Mutation primitives (alias add/remove/rename/repoint, JObj
//! child/next/flags/TRS edits, JObj.alloc) are exposed as the
//! HSDLib-equivalent surface — driving them from Python lets a Blender
//! add-on implement scene-edit pipelines (csx
//! `hsd_import_from_blender.csx` Pass 0–4) without baking project-
//! specific JSON schemas into this library.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use hsdraw_core::accessor::Accessor;
use hsdraw_core::common::JObj as CoreJObj;
use hsdraw_core::dat::RootNode;
use hsdraw_core::gx::JObjFlag;
use hsdraw_core::hsd_struct::{StructRef, ptr_eq};
use hsdraw_core::{export, Dat as CoreDat};
use pyo3::exceptions::{PyIOError, PyKeyError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyBytes;

#[pyfunction]
fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

// =====================================================================
// Dat handle
// =====================================================================

/// Parsed .dat file.  Mirrors HSDLib `HSDRawFile` — exposes `roots`
/// (with mutation), `write()`, and accessor lookup helpers.  Holds a
/// shared `Rc<RefCell<CoreDat>>` so any `Root` / `JObj` view returned
/// from this object stays live as long as the Python user keeps it.
// `unsendable` because the underlying `Rc<RefCell<CoreDat>>` is !Send;
// PyO3's default class bound requires Send+Sync.  Python users only ever
// touch these on the main thread (the GIL is held), so the restriction
// is fine — we just have to opt out of the auto-asserted bound.
#[pyclass(name = "Dat", module = "hsdraw", unsendable)]
struct PyDat {
    inner: Rc<RefCell<CoreDat>>,
}

#[pymethods]
impl PyDat {
    /// Snapshot of every root, in declaration order (public + alias).
    /// Each `Root` shares the underlying struct's `Rc` with the parent
    /// `Dat`, so editing a `Root.data`-derived `JObj` mutates the live
    /// tree and the next `write()` picks up the change.
    fn roots(&self) -> Vec<PyRoot> {
        self.inner
            .borrow()
            .roots
            .iter()
            .map(|r| PyRoot {
                name: r.name.clone(),
                data: r.data.clone(),
            })
            .collect()
    }

    /// Convenience: list of root names (no struct handles).  Cheap to
    /// call repeatedly — used for "does X exist as an alias?" probes.
    fn root_names(&self) -> Vec<String> {
        self.inner.borrow().roots.iter().map(|r| r.name.clone()).collect()
    }

    /// HSDLib `file.Roots.Add(new HSDRootNode { Name = name, Data = data })`.
    /// `data` accepts a `JObj` or `HsdStruct`; everything else raises
    /// `TypeError` to keep the binding's contract obvious.
    #[pyo3(signature = (name, data, /))]
    fn add_root(&self, name: String, data: &Bound<'_, PyAny>) -> PyResult<PyRoot> {
        let s = struct_ref_from_any(data)?;
        self.inner.borrow_mut().add_root(name.clone(), s.clone());
        Ok(PyRoot { name, data: s })
    }

    /// `file.Roots.RemoveAt(file.Roots.FindIndex(r => r.Name == name))`.
    /// Returns `True` if a root was removed, `False` if no such name.
    fn remove_root(&self, name: &str) -> bool {
        self.inner.borrow_mut().remove_root(name)
    }

    /// Rename the first root matching `old`.  Returns `True` on
    /// success, `False` if `old` doesn't exist.  Equivalent to
    /// `file.Roots[i].Name = new` after a name lookup.
    fn rename_root(&self, old: &str, new: String) -> bool {
        self.inner.borrow_mut().rename_root(old, new)
    }

    /// Point an existing root at a different struct.  HSDLib:
    /// `root.Data = newAccessor`.  Returns `True` if `name` exists.
    /// Use this rather than `remove_root` + `add_root` so the alias
    /// keeps its position in the roots list.
    fn repoint_root(&self, name: &str, target: &Bound<'_, PyAny>) -> PyResult<bool> {
        let s = struct_ref_from_any(target)?;
        Ok(self.inner.borrow_mut().repoint_root(name, s))
    }

    /// First root whose data is `Rc::ptr_eq(target)`.  Pythonic
    /// equivalent of `file.Roots.FirstOrDefault(r => r.Data._s == s)`.
    /// Returns `None` if no alias points at `target`.
    #[pyo3(signature = (target, /))]
    fn find_root_for(&self, target: &Bound<'_, PyAny>) -> PyResult<Option<PyRoot>> {
        let s = struct_ref_from_any(target)?;
        let dat = self.inner.borrow();
        Ok(dat.find_root_for(&s).map(|r| PyRoot {
            name: r.name.clone(),
            data: r.data.clone(),
        }))
    }

    /// `scene_data` root if present (every MKGP2 course .dat has it).
    fn scene_data(&self) -> Option<PyRoot> {
        let dat = self.inner.borrow();
        dat.scene_data().map(|r| PyRoot {
            name: r.name.clone(),
            data: r.data.clone(),
        })
    }

    /// Serialize back to .dat bytes.  HSDLib: `file.Save(stream)`.
    /// Honors the same defaults (buffer_align=True, optimize=True);
    /// pass `optimize=False` to disable struct identity dedup + buffer
    /// hash dedup if you're debugging the writer.
    #[pyo3(signature = (optimize=true, buffer_align=true))]
    fn write<'py>(
        &self,
        py: Python<'py>,
        optimize: bool,
        buffer_align: bool,
    ) -> PyResult<Bound<'py, PyBytes>> {
        use hsdraw_core::writer::WriteOptions;
        let opts = WriteOptions { optimize, buffer_align, trim: false };
        let out = self
            .inner
            .borrow()
            .write_with_options(opts)
            .map_err(|e| PyValueError::new_err(format!("write: {:?}", e)))?;
        Ok(PyBytes::new(py, &out))
    }

    fn __repr__(&self) -> String {
        let dat = self.inner.borrow();
        format!("<hsdraw.Dat roots={}>", dat.roots.len())
    }
}

// =====================================================================
// Root  (HSDLib HSDRootNode)
// =====================================================================

/// One entry in `Dat.roots`.  `name` is read-only here — to rename use
/// `Dat.rename_root(old, new)`; the same goes for repointing
/// (`Dat.repoint_root(name, target)`).  Mutation only via the parent
/// `Dat` keeps the roots list consistent and avoids stale-name bugs.
#[pyclass(name = "Root", module = "hsdraw", frozen, unsendable)]
struct PyRoot {
    name: String,
    data: StructRef,
}

#[pymethods]
impl PyRoot {
    #[getter]
    fn name(&self) -> &str {
        &self.name
    }

    /// The struct this alias points at.  Returns an `HsdStruct`; wrap
    /// it with `JObj.from_struct(root.data)` if you need the typed
    /// view.  Identity (`is`) compares the underlying Rc.
    #[getter]
    fn data(&self) -> PyHsdStruct {
        PyHsdStruct { inner: self.data.clone() }
    }

    fn __repr__(&self) -> String {
        format!(
            "<hsdraw.Root name={:?} data_size=0x{:X}>",
            self.name,
            self.data.borrow().len()
        )
    }
}

// =====================================================================
// HsdStruct  (HSDLib HSDStruct)
// =====================================================================

/// Raw struct handle.  Most users won't construct these directly —
/// they come from `Root.data` and `JObj.as_struct()`.  Provided for
/// identity comparison and raw byte inspection only.
#[pyclass(name = "HsdStruct", module = "hsdraw", unsendable)]
struct PyHsdStruct {
    inner: StructRef,
}

#[pymethods]
impl PyHsdStruct {
    /// Bytes occupied by this struct (excludes pointed-at sub-structs).
    fn byte_size(&self) -> usize {
        self.inner.borrow().len()
    }

    /// Snapshot of the raw struct payload.  Pointers in the bytes are
    /// stale (they get rewritten by the writer at save time); use this
    /// for debugging only.
    fn raw<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        let s = self.inner.borrow();
        PyBytes::new(py, s.data())
    }

    /// True iff `self` and `other` share the same underlying Rc.  Same
    /// semantics as `is` in Python — exposed explicitly because PyO3
    /// classes default `__eq__` to value equality on string repr,
    /// which we don't want for struct handles.
    fn __eq__(&self, other: &Self) -> bool {
        ptr_eq(&self.inner, &other.inner)
    }

    fn __hash__(&self) -> isize {
        Rc::as_ptr(&self.inner) as isize
    }

    fn __repr__(&self) -> String {
        format!(
            "<hsdraw.HsdStruct addr=0x{:X} size=0x{:X}>",
            Rc::as_ptr(&self.inner) as usize,
            self.inner.borrow().len()
        )
    }
}

// =====================================================================
// JObj typed view  (HSDLib HSD_JOBJ accessor)
// =====================================================================

/// Typed view onto a 0x40-byte HSD_JOBJ struct.  Properties match
/// HSDLib field names (TX/TY/TZ for translation, RX/RY/RZ for rotation,
/// SX/SY/SZ for scale, plus `flags` and `child`/`next`).  Mutation is
/// in-place on the underlying struct: write `j.tx = 5.0` and the next
/// `Dat.write()` picks it up.
///
/// To allocate a new joint: `JObj.alloc()` returns a fresh 0x40-byte
/// struct with identity scale (1, 1, 1) and zero T/R/flags — equivalent
/// to `new HSD_JOBJ()` followed by `SX = SY = SZ = 1.0f` in csx.
#[pyclass(name = "JObj", module = "hsdraw", unsendable)]
struct PyJObj {
    inner: StructRef,
}

impl PyJObj {
    fn view(&self) -> CoreJObj {
        CoreJObj::from_struct(self.inner.clone())
    }
}

#[pymethods]
impl PyJObj {
    /// Allocate a fresh 0x40-byte HSD_JOBJ.  Matches HSDLib
    /// `new HSD_JOBJ()` plus the explicit identity-scale init csx does.
    #[staticmethod]
    fn alloc() -> Self {
        Self { inner: CoreJObj::allocate_default().0 }
    }

    /// Wrap an existing struct as a JObj typed view.  No bytes are
    /// allocated; the view shares the struct's Rc.  Useful for taking
    /// `Root.data` (an `HsdStruct`) and pulling out joint fields.
    #[staticmethod]
    fn from_struct(s: &PyHsdStruct) -> Self {
        Self { inner: s.inner.clone() }
    }

    /// The underlying `HsdStruct`.  Use this to feed `Dat.add_root` /
    /// `Dat.find_root_for` if your code is generic over the typed view
    /// vs. raw struct distinction.
    fn as_struct(&self) -> PyHsdStruct {
        PyHsdStruct { inner: self.inner.clone() }
    }

    // ----- Hierarchy chain ----------------------------------------
    /// First child joint, or None.  Equivalent to HSDLib
    /// `j.Child` (offset 0x08 reference).
    #[getter]
    fn child(&self) -> Option<PyJObj> {
        self.view().child().map(|c| PyJObj { inner: c.0 })
    }

    /// Next sibling joint in the parent's child chain, or None.
    /// HSDLib: `j.Next` (offset 0x0C reference).
    #[getter]
    fn next(&self) -> Option<PyJObj> {
        self.view().next().map(|n| PyJObj { inner: n.0 })
    }

    /// Set / clear `Child`.  Pass `None` to detach.  No identity check
    /// — the caller is responsible for not creating cycles.
    #[pyo3(signature = (child=None))]
    fn set_child(&self, child: Option<&PyJObj>) {
        self.view().set_child(child.map(|c| c.view()));
    }

    /// Set / clear `Next`.  Same caveat as `set_child`.
    #[pyo3(signature = (next=None))]
    fn set_next(&self, next: Option<&PyJObj>) {
        self.view().set_next(next.map(|n| n.view()));
    }

    // ----- Flags --------------------------------------------------
    /// `JOBJ_FLAG` bits as `u32`.  The name table is intentionally
    /// not exposed — the project consuming this binding should keep
    /// its own enum-name ↔ bits mapping (the table is in HSDLib's
    /// HSD_JOBJ.cs and matches `gx::jobj_flag_names` on the Rust side).
    #[getter]
    fn flags(&self) -> PyResult<u32> {
        self.view()
            .flags()
            .map(|f| f.bits())
            .map_err(map_err)
    }

    #[setter]
    fn set_flags(&self, bits: u32) -> PyResult<()> {
        self.view()
            .set_flags(JObjFlag::from_bits_retain(bits))
            .map_err(map_err)
    }

    // ----- TRS individual --------------------------------------------
    #[getter] fn tx(&self) -> PyResult<f32> { self.view().tx().map_err(map_err) }
    #[getter] fn ty(&self) -> PyResult<f32> { self.view().ty().map_err(map_err) }
    #[getter] fn tz(&self) -> PyResult<f32> { self.view().tz().map_err(map_err) }
    #[getter] fn rx(&self) -> PyResult<f32> { self.view().rx().map_err(map_err) }
    #[getter] fn ry(&self) -> PyResult<f32> { self.view().ry().map_err(map_err) }
    #[getter] fn rz(&self) -> PyResult<f32> { self.view().rz().map_err(map_err) }
    #[getter] fn sx(&self) -> PyResult<f32> { self.view().sx().map_err(map_err) }
    #[getter] fn sy(&self) -> PyResult<f32> { self.view().sy().map_err(map_err) }
    #[getter] fn sz(&self) -> PyResult<f32> { self.view().sz().map_err(map_err) }
    #[setter] fn set_tx(&self, v: f32) -> PyResult<()> { self.view().set_tx(v).map_err(map_err) }
    #[setter] fn set_ty(&self, v: f32) -> PyResult<()> { self.view().set_ty(v).map_err(map_err) }
    #[setter] fn set_tz(&self, v: f32) -> PyResult<()> { self.view().set_tz(v).map_err(map_err) }
    #[setter] fn set_rx(&self, v: f32) -> PyResult<()> { self.view().set_rx(v).map_err(map_err) }
    #[setter] fn set_ry(&self, v: f32) -> PyResult<()> { self.view().set_ry(v).map_err(map_err) }
    #[setter] fn set_rz(&self, v: f32) -> PyResult<()> { self.view().set_rz(v).map_err(map_err) }
    #[setter] fn set_sx(&self, v: f32) -> PyResult<()> { self.view().set_sx(v).map_err(map_err) }
    #[setter] fn set_sy(&self, v: f32) -> PyResult<()> { self.view().set_sy(v).map_err(map_err) }
    #[setter] fn set_sz(&self, v: f32) -> PyResult<()> { self.view().set_sz(v).map_err(map_err) }

    /// Read all 9 TRS components at once as `(tx, ty, tz, rx, ry, rz, sx, sy, sz)`.
    /// Convenience for the common pattern of snapshotting a joint's
    /// local transform.
    fn local_trs(&self) -> PyResult<(f32, f32, f32, f32, f32, f32, f32, f32, f32)> {
        let v = self.view();
        Ok((
            v.tx().map_err(map_err)?,
            v.ty().map_err(map_err)?,
            v.tz().map_err(map_err)?,
            v.rx().map_err(map_err)?,
            v.ry().map_err(map_err)?,
            v.rz().map_err(map_err)?,
            v.sx().map_err(map_err)?,
            v.sy().map_err(map_err)?,
            v.sz().map_err(map_err)?,
        ))
    }

    /// Write all 9 TRS components at once.  Equivalent to setting each
    /// TX..SZ individually but cheaper if you already have a tuple.
    #[allow(clippy::too_many_arguments)]
    #[pyo3(signature = (tx, ty, tz, rx, ry, rz, sx, sy, sz))]
    fn set_local_trs(
        &self,
        tx: f32, ty: f32, tz: f32,
        rx: f32, ry: f32, rz: f32,
        sx: f32, sy: f32, sz: f32,
    ) -> PyResult<()> {
        let v = self.view();
        v.set_tx(tx).map_err(map_err)?;
        v.set_ty(ty).map_err(map_err)?;
        v.set_tz(tz).map_err(map_err)?;
        v.set_rx(rx).map_err(map_err)?;
        v.set_ry(ry).map_err(map_err)?;
        v.set_rz(rz).map_err(map_err)?;
        v.set_sx(sx).map_err(map_err)?;
        v.set_sy(sy).map_err(map_err)?;
        v.set_sz(sz).map_err(map_err)?;
        Ok(())
    }

    // ----- Identity ---------------------------------------------------
    fn __eq__(&self, other: &Self) -> bool {
        ptr_eq(&self.inner, &other.inner)
    }

    fn __hash__(&self) -> isize {
        Rc::as_ptr(&self.inner) as isize
    }

    fn __repr__(&self) -> String {
        format!(
            "<hsdraw.JObj addr=0x{:X}>",
            Rc::as_ptr(&self.inner) as usize
        )
    }
}

// =====================================================================
// Module-level functions (kept for backwards compat with the Phase 4
// API; users can now also do `dat = parse_dat(b); dat.write()` instead
// of going through `write_dat(b)`).
// =====================================================================

/// Parse a .dat from raw bytes.  Returns a mutable `Dat` handle.
#[pyfunction]
#[pyo3(signature = (data, /))]
fn parse_dat(data: &Bound<'_, PyBytes>) -> PyResult<PyDat> {
    let parsed = CoreDat::parse(data.as_bytes())
        .map_err(|e| PyValueError::new_err(format!("parse_dat: {:?}", e)))?;
    Ok(PyDat { inner: Rc::new(RefCell::new(parsed)) })
}

/// Export the scene as a JSON string mirroring the csx golden.  See
/// `docs/python_api.md` for the schema.
#[pyfunction]
#[pyo3(signature = (data, /, source_dat=String::new(), tex_dir=None))]
fn export_scene_json(
    data: &Bound<'_, PyBytes>,
    source_dat: String,
    tex_dir: Option<PathBuf>,
) -> PyResult<String> {
    let bytes = data.as_bytes();
    let parsed = CoreDat::parse(bytes)
        .map_err(|e| PyValueError::new_err(format!("parse_dat: {:?}", e)))?;
    let scene = export::export_scene(&parsed, source_dat, tex_dir.as_deref())
        .map_err(|e| PyValueError::new_err(format!("export_scene: {:?}", e)))?;
    serde_json::to_string(&scene)
        .map_err(|e| PyIOError::new_err(format!("serialize scene: {}", e)))
}

/// Round-trip parse + write.  Standalone form of `Dat.write()` for
/// callers that just want bytes-in, bytes-out without holding a `Dat`.
#[pyfunction]
#[pyo3(signature = (data, /, optimize=true, buffer_align=true))]
fn write_dat<'py>(
    py: Python<'py>,
    data: &Bound<'_, PyBytes>,
    optimize: bool,
    buffer_align: bool,
) -> PyResult<Bound<'py, PyBytes>> {
    use hsdraw_core::writer::WriteOptions;
    let parsed = CoreDat::parse(data.as_bytes())
        .map_err(|e| PyValueError::new_err(format!("parse_dat: {:?}", e)))?;
    let opts = WriteOptions { optimize, buffer_align, trim: false };
    let out = parsed
        .write_with_options(opts)
        .map_err(|e| PyValueError::new_err(format!("write_dat: {:?}", e)))?;
    Ok(PyBytes::new(py, &out))
}

// =====================================================================
// Helpers
// =====================================================================

/// Accept either a `JObj` or `HsdStruct` Python instance and return
/// the shared `StructRef`.  Anything else is `TypeError`.
fn struct_ref_from_any(any: &Bound<'_, PyAny>) -> PyResult<StructRef> {
    if let Ok(j) = any.cast::<PyJObj>() {
        Ok(j.borrow().inner.clone())
    } else if let Ok(s) = any.cast::<PyHsdStruct>() {
        Ok(s.borrow().inner.clone())
    } else {
        Err(PyTypeError::new_err(
            "expected JObj or HsdStruct (got something else)",
        ))
    }
}

#[allow(dead_code)]
fn _root_anchor(_: &RootNode) {} // keep RootNode used so unused-import lint stays happy when this module narrows further

/// Common HsdError → PyErr conversion; we keep the Rust formatting
/// because the inner messages are already user-facing.
fn map_err(e: hsdraw_core::error::HsdError) -> PyErr {
    PyValueError::new_err(format!("{:?}", e))
}

#[allow(dead_code)]
fn _key_err_anchor() -> PyErr {
    PyKeyError::new_err("placeholder")
}

#[pymodule]
fn hsdraw(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(version, m)?)?;
    m.add_function(wrap_pyfunction!(parse_dat, m)?)?;
    m.add_function(wrap_pyfunction!(export_scene_json, m)?)?;
    m.add_function(wrap_pyfunction!(write_dat, m)?)?;
    m.add_class::<PyDat>()?;
    m.add_class::<PyRoot>()?;
    m.add_class::<PyHsdStruct>()?;
    m.add_class::<PyJObj>()?;
    Ok(())
}
