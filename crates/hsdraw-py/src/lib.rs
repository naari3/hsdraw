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
use hsdraw_core::common::{
    DObj as CoreDObj, Image as CoreImage, JObj as CoreJObj, JObjDesc as CoreJObjDesc,
    Lod as CoreLod, MObj as CoreMObj, Material as CoreMaterial, PObj as CorePObj,
    PeDesc as CorePeDesc, SObj as CoreSObj, TObj as CoreTObj,
};
use hsdraw_core::dat::RootNode;
use hsdraw_core::gx::{
    AlphaMap, ColorMap, CoordType, GxAnisotropy, GxTexFilter, GxTexFmt, GxTexGenSrc, GxTexMapId,
    GxTlutFmt, GxWrapMode, JObjFlag, MaterialRenderMode, PObjFlag, TObjFlags,
};
use hsdraw_core::hsd_struct::{StructRef, ptr_eq};
use hsdraw_core::pobj_writer::MeshBuilder as CoreMeshBuilder;
use hsdraw_core::{export, Dat as CoreDat};
use pyo3::exceptions::{PyDeprecationWarning, PyIOError, PyKeyError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict};

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
    /// Allocate an empty `Dat` with a fresh `scene_data` root: SObj →
    /// JOBJDescs[1] → JObjDesc → root JObj placeholder chain.  The root
    /// joint has identity scale and zero TRS — wire children, DObjs,
    /// etc. onto it before saving.  HSDLib equivalent: `new HSDRawFile()`
    /// followed by manual SOBJ tree construction.  Useful for the
    /// vanilla-independent export pipelines (no base .dat to start from).
    #[staticmethod]
    fn alloc_scene_data() -> Self {
        Self {
            inner: Rc::new(RefCell::new(CoreDat::alloc_scene_data())),
        }
    }

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

    /// The `scene_data` root if present — the conventional root name
    /// for HSD-format scene files.  Returns `None` if the .dat has no
    /// such root (e.g. a fighter / character file uses different
    /// conventions).
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

    /// `(offset, target)` pairs for every reference this struct holds,
    /// in ascending offset order.  Mirrors HSDLib `HSDStruct.References`
    /// — needed by callers that have to walk typed sub-structs (e.g.
    /// `HSD_SOBJ.JOBJDescs[]`) without a typed accessor for each layout.
    fn references(&self) -> Vec<(u32, PyHsdStruct)> {
        self.inner
            .borrow()
            .references()
            .iter()
            .map(|(off, target)| (*off, PyHsdStruct { inner: target.clone() }))
            .collect()
    }

    /// Reference at `offset`, or `None` if no reference is set there.
    /// Mirrors HSDLib `HSDStruct.GetReference<T>(offset)` (without the
    /// typed cast — wrap the result in `JObj.from_struct` etc. on the
    /// Python side if you want a typed view).
    fn get_reference(&self, offset: u32) -> Option<PyHsdStruct> {
        self.inner
            .borrow()
            .get_reference(offset)
            .map(|s| PyHsdStruct { inner: s })
    }

    /// Set or clear a reference at `offset`.  Pass `None` to detach.
    /// Mirrors HSDLib `HSDStruct.SetReference(offset, target)`.
    /// Needed for callers that have to repoint deep typed-struct
    /// fields (e.g. `HSD_SOBJ.JOBJDescs[0].RootJoint = new_jobj`)
    /// without a typed accessor for every layout.  Accepts a
    /// `HsdStruct` or any of the typed-view classes (JObj / DObj /
    /// MObj / Material / PeDesc / Pobj) thanks to
    /// `struct_ref_from_any`.
    #[pyo3(signature = (offset, target = None))]
    fn set_reference(&self, offset: u32, target: Option<&Bound<'_, PyAny>>) -> PyResult<()> {
        let resolved = match target {
            None => None,
            Some(any) => Some(struct_ref_from_any(any)?),
        };
        self.inner.borrow_mut().set_reference(offset, resolved);
        Ok(())
    }

    // ----- raw bytes-level setters -------------------------------------
    /// Single-byte write at `offset` (BE-irrelevant since u8 has no
    /// endian).  Mirrors HSDLib `HSDStruct.SetByte(offset, v)`.  Out-
    /// of-bounds writes raise `ValueError`.  Use this rather than
    /// post-write file-byte find/replace patches when a typed setter
    /// for the field doesn't exist yet.
    fn set_u8(&self, offset: u32, value: u8) -> PyResult<()> {
        self.inner.borrow_mut().set_u8(offset, value).map_err(map_err)
    }

    /// Big-endian u16 write at `offset`.  Mirrors HSDLib
    /// `HSDStruct.SetInt16(offset, v)` (the convention in HSD .dat is
    /// always big-endian).
    fn set_u16(&self, offset: u32, value: u16) -> PyResult<()> {
        self.inner.borrow_mut().set_u16(offset, value).map_err(map_err)
    }

    /// Big-endian u32 write at `offset`.  Mirrors HSDLib
    /// `HSDStruct.SetInt32(offset, v)`.
    fn set_u32(&self, offset: u32, value: u32) -> PyResult<()> {
        self.inner.borrow_mut().set_u32(offset, value).map_err(map_err)
    }

    /// Bulk byte write at `offset`.  Errors if the write would land
    /// past the struct's data length.
    fn set_bytes(&self, offset: u32, data: &Bound<'_, PyBytes>) -> PyResult<()> {
        self.inner
            .borrow_mut()
            .set_bytes(offset, data.as_bytes())
            .map_err(map_err)
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

    /// Attach (or detach) a DObj at offset 0x10.  Mirrors HSDLib
    /// `j.Dobj = …`: clears `SPLINE` / `PTCL` flags so the 0x10 union
    /// slot is interpreted as a DObj on the next read.  Pass `None` to
    /// detach.
    #[pyo3(signature = (dobj=None))]
    fn set_dobj(&self, dobj: Option<&PyDObj>) -> PyResult<()> {
        self.view()
            .set_dobj(dobj.map(|d| CoreDObj::from_struct(d.inner.clone())))
            .map_err(map_err)
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

    /// Decoded snapshot of every JObj field as a Python dict.  Mirrors
    /// the HSDLib `HSD_JOBJ` accessor: raw `flags`, presence flags for
    /// child / next / dobj refs, and the local TRS triple.  `addr` is
    /// the underlying struct's Rc pointer cast to int (= identity).
    fn to_dict<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let v = self.view();
        let d = PyDict::new(py);
        d.set_item("addr", Rc::as_ptr(&self.inner) as usize)?;
        d.set_item("flags", v.flags().map_err(map_err)?.bits())?;
        d.set_item("child_present", v.child().is_some())?;
        d.set_item("next_present", v.next().is_some())?;
        d.set_item(
            "dobj_present",
            v.dobj().map_err(map_err)?.is_some(),
        )?;
        d.set_item(
            "rotation",
            (
                v.rx().map_err(map_err)?,
                v.ry().map_err(map_err)?,
                v.rz().map_err(map_err)?,
            ),
        )?;
        d.set_item(
            "scale",
            (
                v.sx().map_err(map_err)?,
                v.sy().map_err(map_err)?,
                v.sz().map_err(map_err)?,
            ),
        )?;
        d.set_item(
            "translation",
            (
                v.tx().map_err(map_err)?,
                v.ty().map_err(map_err)?,
                v.tz().map_err(map_err)?,
            ),
        )?;
        Ok(d)
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
// DObj typed view  (HSDLib HSD_DOBJ accessor)
// =====================================================================

/// Typed view onto a 0x10-byte HSD_DOBJ struct.  Used as the bridge
/// between a JObj and its render data: a DObj owns one MObj (material)
/// and one POBJ (geometry), plus a Next pointer for chaining multiple
/// DObjs off the same JObj.  Construct via `DObj.alloc()`, then
/// `set_mobj` / `set_pobj` and finally `JObj.set_dobj`.
#[pyclass(name = "DObj", module = "hsdraw", unsendable)]
struct PyDObj {
    inner: StructRef,
}

#[pymethods]
impl PyDObj {
    /// Allocate a fresh 0x10-byte HSD_DOBJ.  All fields zero (no
    /// MObj / POBJ / Next yet — set them via the methods below).
    #[staticmethod]
    fn alloc() -> Self {
        Self { inner: CoreDObj::allocate_default().0 }
    }

    /// Wrap an existing struct as a DObj typed view.  No bytes are
    /// allocated; the view shares the struct's Rc.
    #[staticmethod]
    fn from_struct(s: &PyHsdStruct) -> Self {
        Self { inner: s.inner.clone() }
    }

    fn as_struct(&self) -> PyHsdStruct {
        PyHsdStruct { inner: self.inner.clone() }
    }

    /// Next sibling DObj in the parent JObj's chain, or `None`.
    #[getter]
    fn next(&self) -> Option<PyDObj> {
        CoreDObj::from_struct(self.inner.clone())
            .next()
            .map(|n| PyDObj { inner: n.0 })
    }

    /// MObj (material) reference at offset 0x08, or `None` if not set.
    /// Returned as a `HsdStruct` since this binding doesn't carry a
    /// typed MObj wrapper yet (material construction is left to the
    /// caller — see `docs/python_api.md` § Limitations).
    #[getter]
    fn mobj(&self) -> Option<PyHsdStruct> {
        self.inner
            .borrow()
            .get_reference(0x08)
            .map(|s| PyHsdStruct { inner: s })
    }

    /// POBJ reference at offset 0x0C, or `None`.
    #[getter]
    fn pobj(&self) -> Option<PyPObj> {
        self.inner
            .borrow()
            .get_reference(0x0C)
            .map(|s| PyPObj { inner: s })
    }

    /// Attach the next DObj in the chain.  Pass `None` to detach.
    #[pyo3(signature = (next=None))]
    fn set_next(&self, next: Option<&PyDObj>) {
        CoreDObj::from_struct(self.inner.clone())
            .set_next(next.map(|n| CoreDObj::from_struct(n.inner.clone())));
    }

    /// Attach a material.  Accepts either a `MObj` typed view (built
    /// via `MObj.alloc()` / `MObj.alloc_unlit_color(...)`) or a raw
    /// `HsdStruct` — caller's choice depending on whether they're
    /// constructing a fresh material or reusing one pulled out of an
    /// existing course .dat.  Pass `None` to detach.
    #[pyo3(signature = (mobj=None))]
    fn set_mobj(&self, mobj: Option<&Bound<'_, PyAny>>) -> PyResult<()> {
        let target = match mobj {
            Some(b) => Some(struct_ref_from_any(b)?),
            None => None,
        };
        self.inner.borrow_mut().set_reference(0x08, target);
        Ok(())
    }

    /// Attach a POBJ.  Accepts a `Pobj` typed view.  Pass `None` to
    /// detach.
    #[pyo3(signature = (pobj=None))]
    fn set_pobj(&self, pobj: Option<&PyPObj>) {
        CoreDObj::from_struct(self.inner.clone()).set_pobj(
            pobj.map(|p| CorePObj::from_struct(p.inner.clone())),
        );
    }

    fn __eq__(&self, other: &Self) -> bool {
        ptr_eq(&self.inner, &other.inner)
    }

    fn __hash__(&self) -> isize {
        Rc::as_ptr(&self.inner) as isize
    }

    fn __repr__(&self) -> String {
        format!(
            "<hsdraw.DObj addr=0x{:X}>",
            Rc::as_ptr(&self.inner) as usize
        )
    }
}

// =====================================================================
// Pobj typed view  (HSDLib HSD_POBJ accessor)
// =====================================================================

/// Typed view onto a 0x18-byte HSD_POBJ struct.  Returned by
/// `MeshBuilder.build()`.  Phase 1 only exposes inspection getters and
/// the `Next` chain setter; the writer side (mesh data → POBJ) lives
/// behind `MeshBuilder` so the byte-layout details (attribute table /
/// DL bytecode / per-attribute buffer alignment) stay in one place.
#[pyclass(name = "Pobj", module = "hsdraw", unsendable)]
struct PyPObj {
    inner: StructRef,
}

#[pymethods]
impl PyPObj {
    /// Wrap an existing struct as a Pobj typed view.
    #[staticmethod]
    fn from_struct(s: &PyHsdStruct) -> Self {
        Self { inner: s.inner.clone() }
    }

    fn as_struct(&self) -> PyHsdStruct {
        PyHsdStruct { inner: self.inner.clone() }
    }

    /// Next POBJ in the chain, or `None`.
    #[getter]
    fn next(&self) -> Option<PyPObj> {
        self.inner
            .borrow()
            .get_reference(0x04)
            .map(|s| PyPObj { inner: s })
    }

    /// Attach the next POBJ.  Pass `None` to detach.
    #[pyo3(signature = (next=None))]
    fn set_next(&self, next: Option<&PyPObj>) {
        let mut s = self.inner.borrow_mut();
        s.set_reference(0x04, next.map(|n| n.inner.clone()));
    }

    /// `POBJ_FLAG` bits as `u16`.  Bit positions match HSDLib's
    /// `POBJ_FLAG`: ENVELOPE=0x8000, SHAPESET=0x4000, CULLBACK=0x2000,
    /// CULLFRONT=0x1000.  Note that real-world game corpora sometimes
    /// repurpose these bits — most commonly 0x8000 on statically-bound
    /// textured POBJs without an actual envelope-pointer array — which
    /// `MeshBuilder.build` won't emit on its own.  Use the setter
    /// below when you need to overwrite the flag word to match a
    /// specific bit pattern.
    #[getter]
    fn flags(&self) -> PyResult<u16> {
        self.inner.borrow().get_u16(0x0C).map_err(map_err)
    }

    /// Overwrite POBJ.flags (u16 at offset 0x0C).  Use when the flag
    /// value you need doesn't fit HSDLib's canonical `POBJ_FLAG` enum
    /// semantics (e.g. game-specific repurposing of `POBJ_TYPE_MASK` /
    /// `ENVELOPE` bits).  Caller is responsible for keeping the 0x14
    /// reference (envelope pointer array vs. SingleBoundJOBJ vs.
    /// nothing) consistent with whatever bits they're setting.
    #[setter]
    fn set_flags(&self, bits: u16) -> PyResult<()> {
        CorePObj::from_struct(self.inner.clone())
            .set_flags(PObjFlag::from_bits_retain(bits))
            .map_err(map_err)
    }

    /// DL bytecode size in bytes (computed: stored as `bytes/32`).
    #[getter]
    fn display_list_size(&self) -> PyResult<u32> {
        Ok(self.inner.borrow().get_i16(0x0E).map_err(map_err)? as u32 * 32)
    }

    /// Decoded snapshot of every POBJ field as a Python dict.  Mirrors
    /// HSDLib's `HSD_POBJ`: raw `flags` (u16), computed
    /// `display_list_size` (bytes), and presence flags for the four
    /// child slots (`next` 0x04, `attributes_struct` 0x08,
    /// `display_list_buffer` 0x10, `single_bound_jobj` 0x14).
    fn to_dict<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let v = CorePObj::from_struct(self.inner.clone());
        let d = PyDict::new(py);
        d.set_item("addr", Rc::as_ptr(&self.inner) as usize)?;
        d.set_item("flags", v.flags().map_err(map_err)?.bits())?;
        d.set_item(
            "display_list_size",
            v.display_list_size().map_err(map_err)?,
        )?;
        d.set_item("next_present", v.next().is_some())?;
        d.set_item(
            "attributes_struct_present",
            v.attributes_struct().is_some(),
        )?;
        d.set_item(
            "display_list_buffer_present",
            v.display_list_buffer().is_some(),
        )?;
        d.set_item(
            "single_bound_jobj_present",
            v.single_bound_jobj().map_err(map_err)?.is_some(),
        )?;
        Ok(d)
    }

    fn __eq__(&self, other: &Self) -> bool {
        ptr_eq(&self.inner, &other.inner)
    }

    fn __hash__(&self) -> isize {
        Rc::as_ptr(&self.inner) as isize
    }

    fn __repr__(&self) -> String {
        format!(
            "<hsdraw.Pobj addr=0x{:X}>",
            Rc::as_ptr(&self.inner) as usize
        )
    }
}

// =====================================================================
// MObj typed view  (HSDLib HSD_MOBJ accessor)
// =====================================================================

/// Typed view onto a 0x18-byte HSD_MOBJ struct.  Holds the render
/// flags + textures (TObj chain) + Material reference + PE descriptor.
/// Construct via `MObj.alloc()` (empty) or `MObj.alloc_unlit_color()`
/// (one-stop unlit single-color preset), then attach via
/// `DObj.set_mobj`.
#[pyclass(name = "MObj", module = "hsdraw", unsendable)]
struct PyMObj {
    inner: StructRef,
}

#[pymethods]
impl PyMObj {
    /// Allocate a fresh 0x18-byte HSD_MOBJ.  All fields zero (no
    /// render flags / no Material / no Textures / no PE).
    #[staticmethod]
    fn alloc() -> Self {
        Self { inner: CoreMObj::allocate_default().0 }
    }

    /// "Unlit single-color" preset.  Render flags = `CONSTANT |
    /// DIFFUSE`; a fresh `Material` is attached with diffuse RGBA8 =
    /// (r, g, b, a), alpha = 1.0, shininess = 50.0.  No textures, no
    /// PE descriptor.  Useful as the placeholder material for a
    /// brand-new mesh that doesn't have a real material yet.
    ///
    /// Caveat: some HSD-format consumers will not bind any TObj on a
    /// MObj that lacks `LIGHTMAP_DIFFUSE` (a TObj-side flag), so this
    /// preset isn't suitable for textured rendering even after a TObj
    /// is later attached.  Use `MObj.alloc_textured(material, image)`
    /// for the textured-render preset.
    #[staticmethod]
    fn alloc_unlit_color(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { inner: CoreMObj::allocate_unlit_color(r, g, b, a).0 }
    }

    /// "Textured lit" preset: one-call wiring of MObj + a fresh TObj
    /// pointing at the supplied `material` and `image`.  Sets render
    /// flags to `CONSTANT | DIFFUSE | TEX0 | ALPHA_MAT`, allocates a
    /// TObj with the field values widely seen on textured POBJs in
    /// vanilla HSD course corpora (TG_TEX0, MODULATE color/alpha op,
    /// REPEAT wrap, LINEAR mag, blending=1.0, LIGHTMAP_DIFFUSE on),
    /// and attaches the supplied Image.
    ///
    /// Caller responsibility:
    ///   - Build the `Material` first (e.g. `Material.new(amb=..., dif=...)`).
    ///   - Build the `Image` first: `Image.alloc()` → set `width` /
    ///     `height` / `format` → `set_image_data_bytes(...)`.
    ///   - Attach a `PeDesc` separately via `set_pe_desc(...)` if
    ///     alpha-test / blend-mode tweaks are needed.
    ///   - Attach a `Tlut` via `mobj.textures.set_tlut_data(...)` for
    ///     paletted image formats (`CI4` / `CI8` / `CI14X2`).
    #[staticmethod]
    fn alloc_textured(material: &PyMaterial, image: &PyImage) -> Self {
        let mat = CoreMaterial::from_struct(material.inner.clone());
        let img = CoreImage::from_struct(image.inner.clone());
        Self { inner: CoreMObj::allocate_textured(mat, img).0 }
    }

    #[staticmethod]
    fn from_struct(s: &PyHsdStruct) -> Self {
        Self { inner: s.inner.clone() }
    }

    fn as_struct(&self) -> PyHsdStruct {
        PyHsdStruct { inner: self.inner.clone() }
    }

    /// `RENDER_MODE` bits (matches HSDLib's `RENDER_MODE` enum: see
    /// `crates/hsdraw-core/src/gx.rs`'s `MaterialRenderMode` constants).
    #[getter]
    fn render_flags(&self) -> PyResult<u32> {
        CoreMObj::from_struct(self.inner.clone())
            .render_flags()
            .map(|f| f.bits())
            .map_err(map_err)
    }

    #[setter]
    fn set_render_flags(&self, bits: u32) -> PyResult<()> {
        CoreMObj::from_struct(self.inner.clone())
            .set_render_flags(MaterialRenderMode::from_bits_retain(bits))
            .map_err(map_err)
    }

    /// Attached Material, or `None`.
    #[getter]
    fn material(&self) -> Option<PyMaterial> {
        self.inner
            .borrow()
            .get_reference(0x0C)
            .map(|s| PyMaterial { inner: s })
    }

    /// Attached PE descriptor, or `None`.
    #[getter]
    fn pe_desc(&self) -> Option<PyPeDesc> {
        self.inner
            .borrow()
            .get_reference(0x14)
            .map(|s| PyPeDesc { inner: s })
    }

    /// Attached TObj chain head, or `None`.
    #[getter]
    fn textures(&self) -> Option<PyTObj> {
        self.inner
            .borrow()
            .get_reference(0x08)
            .map(|s| PyTObj { inner: s })
    }

    #[pyo3(signature = (material=None))]
    fn set_material(&self, material: Option<&PyMaterial>) {
        CoreMObj::from_struct(self.inner.clone()).set_material(
            material.map(|m| CoreMaterial::from_struct(m.inner.clone())),
        );
    }

    #[pyo3(signature = (pe=None))]
    fn set_pe_desc(&self, pe: Option<&PyPeDesc>) {
        CoreMObj::from_struct(self.inner.clone()).set_pe_desc(
            pe.map(|p| CorePeDesc::from_struct(p.inner.clone())),
        );
    }

    /// Set the TObj chain head.  Accepts a `TObj` typed view or any
    /// other struct handle (raw `HsdStruct`, etc.); `None` detaches.
    #[pyo3(signature = (tobj=None))]
    fn set_textures(&self, tobj: Option<&Bound<'_, PyAny>>) -> PyResult<()> {
        let target = match tobj {
            Some(b) => Some(struct_ref_from_any(b)?),
            None => None,
        };
        self.inner.borrow_mut().set_reference(0x08, target);
        Ok(())
    }

    /// Decoded snapshot of every MObj field as a Python dict.  Carries
    /// the raw `render_flags` (HSDLib `RENDER_MODE` bits) plus presence
    /// flags for the three child refs (`material` 0x0C, `textures` 0x08
    /// / TObj chain head, `pe_desc` 0x14).  Identity is `addr` (Rc ptr).
    fn to_dict<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let v = CoreMObj::from_struct(self.inner.clone());
        let d = PyDict::new(py);
        d.set_item("addr", Rc::as_ptr(&self.inner) as usize)?;
        d.set_item(
            "render_flags",
            v.render_flags().map_err(map_err)?.bits(),
        )?;
        d.set_item("material_present", v.material().is_some())?;
        d.set_item("textures_present", v.textures().is_some())?;
        d.set_item("pe_desc_present", v.pe_desc().is_some())?;
        Ok(d)
    }

    fn __eq__(&self, other: &Self) -> bool {
        ptr_eq(&self.inner, &other.inner)
    }

    fn __hash__(&self) -> isize {
        Rc::as_ptr(&self.inner) as isize
    }

    fn __repr__(&self) -> String {
        format!(
            "<hsdraw.MObj addr=0x{:X}>",
            Rc::as_ptr(&self.inner) as usize
        )
    }
}

// =====================================================================
// Material typed view  (HSDLib HSD_Material accessor)
// =====================================================================

/// Typed view onto a 0x14-byte HSD_Material struct.  Holds ambient /
/// diffuse / specular RGBA8 + alpha (f32) + shininess (f32).  Attach
/// via `MObj.set_material`.
#[pyclass(name = "Material", module = "hsdraw", unsendable)]
struct PyMaterial {
    inner: StructRef,
}

#[pymethods]
impl PyMaterial {
    /// Allocate a fresh 0x14-byte Material with all-zero fields.
    #[staticmethod]
    fn alloc() -> Self {
        Self { inner: CoreMaterial::allocate_default().0 }
    }

    /// One-call constructor: allocate + set every field in one shot.
    /// All arguments are keyword-only with sensible defaults so callers
    /// can fill in just the fields they care about.  Equivalent to
    /// `Material.alloc()` + 5 setter calls.
    ///
    /// - `amb` / `dif` / `spc`: RGBA8 4-tuples (`(r, g, b, a)` u8).
    /// - `alpha`: f32 multiplier into every TEV stage's `RAS` source α
    ///   (sensible range `[0.0, 1.0]`).
    /// - `shininess`: f32 Phong-cosine exponent for the specular
    ///   highlight (sensible range roughly `[1.0, 200.0]`; default 50).
    #[staticmethod]
    #[pyo3(signature = (
        *,
        amb = (0, 0, 0, 0),
        dif = (0xFF, 0xFF, 0xFF, 0xFF),
        spc = (0xFF, 0xFF, 0xFF, 0xFF),
        alpha = 1.0,
        shininess = 50.0,
    ))]
    fn new(
        amb: (u8, u8, u8, u8),
        dif: (u8, u8, u8, u8),
        spc: (u8, u8, u8, u8),
        alpha: f32,
        shininess: f32,
    ) -> PyResult<Self> {
        let mat = CoreMaterial::allocate(
            [amb.0, amb.1, amb.2, amb.3],
            [dif.0, dif.1, dif.2, dif.3],
            [spc.0, spc.1, spc.2, spc.3],
            alpha,
            shininess,
        )
        .map_err(map_err)?;
        Ok(Self { inner: mat.0 })
    }

    #[staticmethod]
    fn from_struct(s: &PyHsdStruct) -> Self {
        Self { inner: s.inner.clone() }
    }

    fn as_struct(&self) -> PyHsdStruct {
        PyHsdStruct { inner: self.inner.clone() }
    }

    #[getter]
    fn amb_rgba(&self) -> PyResult<(u8, u8, u8, u8)> {
        let v = CoreMaterial::from_struct(self.inner.clone())
            .amb_rgba()
            .map_err(map_err)?;
        Ok((v[0], v[1], v[2], v[3]))
    }

    #[setter]
    fn set_amb_rgba(&self, rgba: (u8, u8, u8, u8)) -> PyResult<()> {
        CoreMaterial::from_struct(self.inner.clone())
            .set_amb_rgba([rgba.0, rgba.1, rgba.2, rgba.3])
            .map_err(map_err)
    }

    #[getter]
    fn dif_rgba(&self) -> PyResult<(u8, u8, u8, u8)> {
        let v = CoreMaterial::from_struct(self.inner.clone())
            .dif_rgba()
            .map_err(map_err)?;
        Ok((v[0], v[1], v[2], v[3]))
    }

    #[setter]
    fn set_dif_rgba(&self, rgba: (u8, u8, u8, u8)) -> PyResult<()> {
        CoreMaterial::from_struct(self.inner.clone())
            .set_dif_rgba([rgba.0, rgba.1, rgba.2, rgba.3])
            .map_err(map_err)
    }

    #[getter]
    fn spc_rgba(&self) -> PyResult<(u8, u8, u8, u8)> {
        let v = CoreMaterial::from_struct(self.inner.clone())
            .spc_rgba()
            .map_err(map_err)?;
        Ok((v[0], v[1], v[2], v[3]))
    }

    #[setter]
    fn set_spc_rgba(&self, rgba: (u8, u8, u8, u8)) -> PyResult<()> {
        CoreMaterial::from_struct(self.inner.clone())
            .set_spc_rgba([rgba.0, rgba.1, rgba.2, rgba.3])
            .map_err(map_err)
    }

    #[getter]
    fn alpha(&self) -> PyResult<f32> {
        CoreMaterial::from_struct(self.inner.clone())
            .alpha()
            .map_err(map_err)
    }

    #[setter]
    fn set_alpha(&self, v: f32) -> PyResult<()> {
        CoreMaterial::from_struct(self.inner.clone())
            .set_alpha(v)
            .map_err(map_err)
    }

    #[getter]
    fn shininess(&self) -> PyResult<f32> {
        CoreMaterial::from_struct(self.inner.clone())
            .shininess()
            .map_err(map_err)
    }

    #[setter]
    fn set_shininess(&self, v: f32) -> PyResult<()> {
        CoreMaterial::from_struct(self.inner.clone())
            .set_shininess(v)
            .map_err(map_err)
    }

    fn __eq__(&self, other: &Self) -> bool {
        ptr_eq(&self.inner, &other.inner)
    }

    fn __hash__(&self) -> isize {
        Rc::as_ptr(&self.inner) as isize
    }

    fn __repr__(&self) -> String {
        format!(
            "<hsdraw.Material addr=0x{:X}>",
            Rc::as_ptr(&self.inner) as usize
        )
    }
}

// =====================================================================
// PeDesc typed view  (HSDLib HSD_PEDesc accessor)
// =====================================================================

/// Typed view onto a 0xC-byte HSD_PEDesc (Pixel-process Engine
/// descriptor).  Holds blend mode + factors + depth/alpha test setup.
/// All fields are u8 — refer to `HSDRaw/Common/HSD_MOBJ.cs` /
/// `HSDRaw/GX/Enums.cs` for the GX enum values.  Attach via
/// `MObj.set_pe_desc`.
#[pyclass(name = "PeDesc", module = "hsdraw", unsendable)]
struct PyPeDesc {
    inner: StructRef,
}

#[pymethods]
impl PyPeDesc {
    #[staticmethod]
    fn alloc() -> Self {
        Self { inner: CorePeDesc::allocate_default().0 }
    }

    #[staticmethod]
    fn from_struct(s: &PyHsdStruct) -> Self {
        Self { inner: s.inner.clone() }
    }

    fn as_struct(&self) -> PyHsdStruct {
        PyHsdStruct { inner: self.inner.clone() }
    }

    #[getter] fn flags(&self) -> PyResult<u8> { CorePeDesc::from_struct(self.inner.clone()).flags().map_err(map_err) }
    #[getter] fn alpha_ref0(&self) -> PyResult<u8> { CorePeDesc::from_struct(self.inner.clone()).alpha_ref0().map_err(map_err) }
    #[getter] fn alpha_ref1(&self) -> PyResult<u8> { CorePeDesc::from_struct(self.inner.clone()).alpha_ref1().map_err(map_err) }
    #[getter] fn destination_alpha(&self) -> PyResult<u8> { CorePeDesc::from_struct(self.inner.clone()).destination_alpha().map_err(map_err) }
    #[getter] fn blend_mode(&self) -> PyResult<u8> { CorePeDesc::from_struct(self.inner.clone()).blend_mode().map_err(map_err) }
    #[getter] fn src_factor(&self) -> PyResult<u8> { CorePeDesc::from_struct(self.inner.clone()).src_factor().map_err(map_err) }
    #[getter] fn dst_factor(&self) -> PyResult<u8> { CorePeDesc::from_struct(self.inner.clone()).dst_factor().map_err(map_err) }
    #[getter] fn blend_op(&self) -> PyResult<u8> { CorePeDesc::from_struct(self.inner.clone()).blend_op().map_err(map_err) }
    #[getter] fn depth_function(&self) -> PyResult<u8> { CorePeDesc::from_struct(self.inner.clone()).depth_function().map_err(map_err) }
    #[getter] fn alpha_comp0(&self) -> PyResult<u8> { CorePeDesc::from_struct(self.inner.clone()).alpha_comp0().map_err(map_err) }
    #[getter] fn alpha_op(&self) -> PyResult<u8> { CorePeDesc::from_struct(self.inner.clone()).alpha_op().map_err(map_err) }
    #[getter] fn alpha_comp1(&self) -> PyResult<u8> { CorePeDesc::from_struct(self.inner.clone()).alpha_comp1().map_err(map_err) }

    #[setter] fn set_flags(&self, v: u8) -> PyResult<()> { CorePeDesc::from_struct(self.inner.clone()).set_flags(v).map_err(map_err) }
    #[setter] fn set_alpha_ref0(&self, v: u8) -> PyResult<()> { CorePeDesc::from_struct(self.inner.clone()).set_alpha_ref0(v).map_err(map_err) }
    #[setter] fn set_alpha_ref1(&self, v: u8) -> PyResult<()> { CorePeDesc::from_struct(self.inner.clone()).set_alpha_ref1(v).map_err(map_err) }
    #[setter] fn set_destination_alpha(&self, v: u8) -> PyResult<()> { CorePeDesc::from_struct(self.inner.clone()).set_destination_alpha(v).map_err(map_err) }
    #[setter] fn set_blend_mode(&self, v: u8) -> PyResult<()> { CorePeDesc::from_struct(self.inner.clone()).set_blend_mode(v).map_err(map_err) }
    #[setter] fn set_src_factor(&self, v: u8) -> PyResult<()> { CorePeDesc::from_struct(self.inner.clone()).set_src_factor(v).map_err(map_err) }
    #[setter] fn set_dst_factor(&self, v: u8) -> PyResult<()> { CorePeDesc::from_struct(self.inner.clone()).set_dst_factor(v).map_err(map_err) }
    #[setter] fn set_blend_op(&self, v: u8) -> PyResult<()> { CorePeDesc::from_struct(self.inner.clone()).set_blend_op(v).map_err(map_err) }
    #[setter] fn set_depth_function(&self, v: u8) -> PyResult<()> { CorePeDesc::from_struct(self.inner.clone()).set_depth_function(v).map_err(map_err) }
    #[setter] fn set_alpha_comp0(&self, v: u8) -> PyResult<()> { CorePeDesc::from_struct(self.inner.clone()).set_alpha_comp0(v).map_err(map_err) }
    #[setter] fn set_alpha_op(&self, v: u8) -> PyResult<()> { CorePeDesc::from_struct(self.inner.clone()).set_alpha_op(v).map_err(map_err) }
    #[setter] fn set_alpha_comp1(&self, v: u8) -> PyResult<()> { CorePeDesc::from_struct(self.inner.clone()).set_alpha_comp1(v).map_err(map_err) }

    fn __eq__(&self, other: &Self) -> bool {
        ptr_eq(&self.inner, &other.inner)
    }

    fn __hash__(&self) -> isize {
        Rc::as_ptr(&self.inner) as isize
    }

    fn __repr__(&self) -> String {
        format!(
            "<hsdraw.PeDesc addr=0x{:X}>",
            Rc::as_ptr(&self.inner) as usize
        )
    }
}

// =====================================================================
// SObj typed view  (HSDLib HSD_SOBJ accessor)
// =====================================================================

/// Typed view onto a 0x10-byte HSD_SOBJ struct.  HSDLib's "scene
/// object" — root of the scene_data root — owns the JOBJDescs[] array
/// (one entry per logical model root) plus camera / light / fog refs
/// that this binding doesn't expose yet (course .dat doesn't exercise
/// those slots).  Construct via `SObj.alloc()` or extract from an
/// existing .dat via `SObj.from_struct(root.data)`.
#[pyclass(name = "SObj", module = "hsdraw", unsendable)]
struct PySObj {
    inner: StructRef,
}

#[pymethods]
impl PySObj {
    /// Allocate a fresh 0x10-byte HSD_SOBJ.  All fields zero (no
    /// JOBJDescs[] attached).  Pair with `set_jobj_descs([...])` to
    /// install one or more JObjDescs.
    #[staticmethod]
    fn alloc() -> Self {
        Self { inner: CoreSObj::allocate_default().0 }
    }

    #[staticmethod]
    fn from_struct(s: &PyHsdStruct) -> Self {
        Self { inner: s.inner.clone() }
    }

    fn as_struct(&self) -> PyHsdStruct {
        PyHsdStruct { inner: self.inner.clone() }
    }

    /// Snapshot of every JObjDesc reachable through this SObj's
    /// JOBJDescs[] array.  Empty if no array is attached yet.
    fn jobj_descs(&self) -> Vec<PyJObjDesc> {
        CoreSObj::from_struct(self.inner.clone())
            .jobj_descs()
            .into_iter()
            .map(|d| PyJObjDesc { inner: d.0 })
            .collect()
    }

    /// The raw JOBJDescs[] array struct, or `None`.  Returned as
    /// `HsdStruct` since this is just a flat array of JObjDesc pointers
    /// (`HSDNullPointerArrayAccessor<HSD_JOBJDesc>` in HSDLib).
    fn jobj_descs_array(&self) -> Option<PyHsdStruct> {
        CoreSObj::from_struct(self.inner.clone())
            .jobj_descs_array()
            .map(|s| PyHsdStruct { inner: s })
    }

    /// Replace the JOBJDescs[] array slot.  Pass a list of JObjDescs and
    /// the binding builds the underlying NullPointerArrayAccessor struct
    /// for you (4 bytes per entry plus a 4-byte NULL terminator).
    fn set_jobj_descs(&self, descs: Vec<PyRef<'_, PyJObjDesc>>) {
        let core: Vec<CoreJObjDesc> = descs
            .iter()
            .map(|d| CoreJObjDesc::from_struct(d.inner.clone()))
            .collect();
        let arr = hsdraw_core::common::build_jobj_descs_array(&core);
        CoreSObj::from_struct(self.inner.clone()).set_jobj_descs_array(Some(arr));
    }

    /// Decoded snapshot of every SObj field as a Python dict.  Reports
    /// the count of `JOBJDescs[]` entries (HSDLib's
    /// `HSDNullPointerArrayAccessor` walk) plus a presence flag for the
    /// underlying array struct.  Camera / light / fog slots aren't
    /// surfaced here (this binding doesn't expose them yet).
    fn to_dict<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let v = CoreSObj::from_struct(self.inner.clone());
        let d = PyDict::new(py);
        d.set_item("addr", Rc::as_ptr(&self.inner) as usize)?;
        d.set_item("jobj_descs_count", v.jobj_descs().len())?;
        d.set_item(
            "jobj_descs_array_present",
            v.jobj_descs_array().is_some(),
        )?;
        Ok(d)
    }

    fn __eq__(&self, other: &Self) -> bool {
        ptr_eq(&self.inner, &other.inner)
    }

    fn __hash__(&self) -> isize {
        Rc::as_ptr(&self.inner) as isize
    }

    fn __repr__(&self) -> String {
        format!(
            "<hsdraw.SObj addr=0x{:X}>",
            Rc::as_ptr(&self.inner) as usize
        )
    }
}

// =====================================================================
// JObjDesc typed view  (HSDLib HSD_JOBJDesc accessor)
// =====================================================================

/// Typed view onto a 0x10-byte HSD_JOBJDesc struct.  Holds one root
/// joint reference (offset 0x00) plus three anim slots (0x04..0x0C)
/// that this binding doesn't expose (course .dat anim is read-only for
/// now).  Construct via `JObjDesc.alloc()` then `set_root_joint(j)`.
#[pyclass(name = "JObjDesc", module = "hsdraw", unsendable)]
struct PyJObjDesc {
    inner: StructRef,
}

#[pymethods]
impl PyJObjDesc {
    /// Allocate a fresh 0x10-byte HSD_JOBJDesc.  All fields zero.
    #[staticmethod]
    fn alloc() -> Self {
        Self { inner: CoreJObjDesc::allocate_default().0 }
    }

    #[staticmethod]
    fn from_struct(s: &PyHsdStruct) -> Self {
        Self { inner: s.inner.clone() }
    }

    fn as_struct(&self) -> PyHsdStruct {
        PyHsdStruct { inner: self.inner.clone() }
    }

    /// Root joint of this descriptor, or `None`.
    #[getter]
    fn root_joint(&self) -> Option<PyJObj> {
        CoreJObjDesc::from_struct(self.inner.clone())
            .root_joint()
            .map(|j| PyJObj { inner: j.0 })
    }

    /// Set / clear the root joint.  Pass `None` to detach.
    #[pyo3(signature = (j=None))]
    fn set_root_joint(&self, j: Option<&PyJObj>) {
        CoreJObjDesc::from_struct(self.inner.clone())
            .set_root_joint(j.map(|jj| CoreJObj::from_struct(jj.inner.clone())));
    }

    fn __eq__(&self, other: &Self) -> bool {
        ptr_eq(&self.inner, &other.inner)
    }

    fn __hash__(&self) -> isize {
        Rc::as_ptr(&self.inner) as isize
    }

    fn __repr__(&self) -> String {
        format!(
            "<hsdraw.JObjDesc addr=0x{:X}>",
            Rc::as_ptr(&self.inner) as usize
        )
    }
}

// =====================================================================
// TObj typed view  (HSDLib HSD_TOBJ accessor)
// =====================================================================

/// Typed view onto a 0x5C-byte HSD_TOBJ struct.  Holds the texture
/// slot id, transform (rotation / scale / translation), wrap modes,
/// flags + coord/color/alpha operation nibbles, blending factor, mag
/// filter, and references to the `Image` (raw GX-encoded payload) and
/// `Tlut` (palette, where applicable).  Construct via `TObj.alloc()`,
/// then attach a `Image` via `set_image_data` and wire into a `MObj`
/// via `MObj.set_textures(tobj)`.  Multiple TObjs can be chained via
/// `set_next` for textures 0..N on the same material.
#[pyclass(name = "TObj", module = "hsdraw", unsendable)]
struct PyTObj {
    inner: StructRef,
}

#[pymethods]
impl PyTObj {
    /// Allocate a fresh 0x5C-byte HSD_TOBJ.  All fields zero (no
    /// image, no TLUT, identity-zero TRS, wrap=CLAMP).
    #[staticmethod]
    fn alloc() -> Self {
        Self { inner: CoreTObj::allocate_default().0 }
    }

    #[staticmethod]
    fn from_struct(s: &PyHsdStruct) -> Self {
        Self { inner: s.inner.clone() }
    }

    fn as_struct(&self) -> PyHsdStruct {
        PyHsdStruct { inner: self.inner.clone() }
    }

    // ----- chain --------------------------------------------------
    /// Next TObj in the texture chain, or `None`.
    #[getter]
    fn next(&self) -> Option<PyTObj> {
        CoreTObj::from_struct(self.inner.clone())
            .next()
            .map(|t| PyTObj { inner: t.0 })
    }

    #[pyo3(signature = (next=None))]
    fn set_next(&self, next: Option<&PyTObj>) {
        CoreTObj::from_struct(self.inner.clone())
            .set_next(next.map(|n| CoreTObj::from_struct(n.inner.clone())));
    }

    // ----- texture slot id ----------------------------------------
    /// `GX_TEXMAP*` value as `u32` (0..7 = TEXMAP0..7).
    #[getter]
    fn tex_map_id(&self) -> PyResult<u32> {
        CoreTObj::from_struct(self.inner.clone())
            .tex_map_id()
            .map(u32::from)
            .map_err(map_err)
    }

    #[setter]
    fn set_tex_map_id(&self, v: u32) -> PyResult<()> {
        CoreTObj::from_struct(self.inner.clone())
            .set_tex_map_id(GxTexMapId::from(v))
            .map_err(map_err)
    }

    /// `GXTexGenSrc` (offset 0x0C) as `u32`.  Controls which vertex
    /// source feeds the texture-coord generator: 0=`GX_TG_POS` (world
    /// position), 4=`GX_TG_TEX0` (POBJ TEX0 attribute), etc.  See
    /// `hsdraw_core::gx::GxTexGenSrc` for the full enum.  Default
    /// (0=`GX_TG_POS`) usually wants overriding to `GX_TG_TEX0` for
    /// regular UV-mapped textures.
    #[getter]
    fn tex_gen_src(&self) -> PyResult<u32> {
        CoreTObj::from_struct(self.inner.clone())
            .tex_gen_src()
            .map(u32::from)
            .map_err(map_err)
    }

    #[setter]
    fn set_tex_gen_src(&self, v: u32) -> PyResult<()> {
        CoreTObj::from_struct(self.inner.clone())
            .set_tex_gen_src(GxTexGenSrc::from(v))
            .map_err(map_err)
    }

    // ----- transform triples --------------------------------------
    fn set_rotation(&self, rx: f32, ry: f32, rz: f32) -> PyResult<()> {
        CoreTObj::from_struct(self.inner.clone())
            .set_rotation(rx, ry, rz)
            .map_err(map_err)
    }

    fn set_scale(&self, sx: f32, sy: f32, sz: f32) -> PyResult<()> {
        CoreTObj::from_struct(self.inner.clone())
            .set_scale(sx, sy, sz)
            .map_err(map_err)
    }

    fn set_translation(&self, tx: f32, ty: f32, tz: f32) -> PyResult<()> {
        CoreTObj::from_struct(self.inner.clone())
            .set_translation(tx, ty, tz)
            .map_err(map_err)
    }

    // ----- wrap / repeat ------------------------------------------
    /// `GX_WRAPMODE` value (0=CLAMP, 1=REPEAT, 2=MIRROR).
    #[getter]
    fn wrap_s(&self) -> PyResult<u32> {
        CoreTObj::from_struct(self.inner.clone())
            .wrap_s()
            .map(u32::from)
            .map_err(map_err)
    }

    #[setter]
    fn set_wrap_s(&self, v: u32) -> PyResult<()> {
        CoreTObj::from_struct(self.inner.clone())
            .set_wrap_s(GxWrapMode::from(v))
            .map_err(map_err)
    }

    #[getter]
    fn wrap_t(&self) -> PyResult<u32> {
        CoreTObj::from_struct(self.inner.clone())
            .wrap_t()
            .map(u32::from)
            .map_err(map_err)
    }

    #[setter]
    fn set_wrap_t(&self, v: u32) -> PyResult<()> {
        CoreTObj::from_struct(self.inner.clone())
            .set_wrap_t(GxWrapMode::from(v))
            .map_err(map_err)
    }

    #[getter]
    fn repeat_s(&self) -> PyResult<u8> {
        CoreTObj::from_struct(self.inner.clone()).repeat_s().map_err(map_err)
    }

    #[setter]
    fn set_repeat_s(&self, v: u8) -> PyResult<()> {
        CoreTObj::from_struct(self.inner.clone()).set_repeat_s(v).map_err(map_err)
    }

    #[getter]
    fn repeat_t(&self) -> PyResult<u8> {
        CoreTObj::from_struct(self.inner.clone()).repeat_t().map_err(map_err)
    }

    #[setter]
    fn set_repeat_t(&self, v: u8) -> PyResult<()> {
        CoreTObj::from_struct(self.inner.clone()).set_repeat_t(v).map_err(map_err)
    }

    // ----- flags + nibbles ----------------------------------------
    /// Raw 0x40 word (`TObjFlags` bits, plus the `coord_type` /
    /// `color_operation` / `alpha_operation` nibbles HSDLib packs into
    /// the same u32).  Setter clobbers all 32 bits — use the per-
    /// nibble setters below for in-place updates.
    #[getter]
    fn flags(&self) -> PyResult<u32> {
        CoreTObj::from_struct(self.inner.clone())
            .flags()
            .map(|f| f.bits())
            .map_err(map_err)
    }

    #[setter]
    fn set_flags(&self, v: u32) -> PyResult<()> {
        CoreTObj::from_struct(self.inner.clone())
            .set_flags(TObjFlags::from_bits_retain(v))
            .map_err(map_err)
    }

    fn set_coord_type(&self, coord: u32) -> PyResult<()> {
        CoreTObj::from_struct(self.inner.clone())
            .set_coord_type(CoordType::from(coord))
            .map_err(map_err)
    }

    // ----- named TObjFlags bit setters (RMW-preserving) ----------------
    /// `LIGHTMAP_DIFFUSE` (bit 4).  RMW preserves every other bit and
    /// the coord_type / color_op / alpha_op nibbles — call this rather
    /// than `tobj.flags |= 0x10` so the nibbles aren't accidentally
    /// stomped.  Some renderers skip texture sampling for TObjs
    /// without this bit, so it's worth setting on freshly-allocated
    /// TObjs unless the texture explicitly shouldn't drive diffuse.
    #[pyo3(signature = (on, /))]
    fn set_lightmap_diffuse(&self, on: bool) -> PyResult<()> {
        CoreTObj::from_struct(self.inner.clone())
            .set_lightmap_diffuse(on)
            .map_err(map_err)
    }
    fn is_lightmap_diffuse(&self) -> PyResult<bool> {
        CoreTObj::from_struct(self.inner.clone())
            .is_lightmap_diffuse()
            .map_err(map_err)
    }

    /// `LIGHTMAP_SPECULAR` (bit 5).  Same RMW semantics as
    /// `set_lightmap_diffuse`.
    #[pyo3(signature = (on, /))]
    fn set_lightmap_specular(&self, on: bool) -> PyResult<()> {
        CoreTObj::from_struct(self.inner.clone())
            .set_lightmap_specular(on)
            .map_err(map_err)
    }
    fn is_lightmap_specular(&self) -> PyResult<bool> {
        CoreTObj::from_struct(self.inner.clone())
            .is_lightmap_specular()
            .map_err(map_err)
    }

    /// `LIGHTMAP_AMBIENT` (bit 6).
    #[pyo3(signature = (on, /))]
    fn set_lightmap_ambient(&self, on: bool) -> PyResult<()> {
        CoreTObj::from_struct(self.inner.clone())
            .set_lightmap_ambient(on)
            .map_err(map_err)
    }
    fn is_lightmap_ambient(&self) -> PyResult<bool> {
        CoreTObj::from_struct(self.inner.clone())
            .is_lightmap_ambient()
            .map_err(map_err)
    }

    /// `LIGHTMAP_EXT` (bit 7).
    #[pyo3(signature = (on, /))]
    fn set_lightmap_ext(&self, on: bool) -> PyResult<()> {
        CoreTObj::from_struct(self.inner.clone())
            .set_lightmap_ext(on)
            .map_err(map_err)
    }
    fn is_lightmap_ext(&self) -> PyResult<bool> {
        CoreTObj::from_struct(self.inner.clone())
            .is_lightmap_ext()
            .map_err(map_err)
    }

    /// `LIGHTMAP_SHADOW` (bit 8).
    #[pyo3(signature = (on, /))]
    fn set_lightmap_shadow(&self, on: bool) -> PyResult<()> {
        CoreTObj::from_struct(self.inner.clone())
            .set_lightmap_shadow(on)
            .map_err(map_err)
    }
    fn is_lightmap_shadow(&self) -> PyResult<bool> {
        CoreTObj::from_struct(self.inner.clone())
            .is_lightmap_shadow()
            .map_err(map_err)
    }

    /// `BUMP` (bit 24).
    #[pyo3(signature = (on, /))]
    fn set_bump(&self, on: bool) -> PyResult<()> {
        CoreTObj::from_struct(self.inner.clone())
            .set_bump(on)
            .map_err(map_err)
    }
    fn is_bump(&self) -> PyResult<bool> {
        CoreTObj::from_struct(self.inner.clone())
            .is_bump()
            .map_err(map_err)
    }

    /// Generic single-bit RMW for any `TObjFlags` mask.  Foundation
    /// for the named setters above; exposed for callers that want to
    /// toggle a custom mask without going through the raw u32 path.
    #[pyo3(signature = (mask, on))]
    fn set_flag_bit(&self, mask: u32, on: bool) -> PyResult<()> {
        CoreTObj::from_struct(self.inner.clone())
            .set_flag_bit(TObjFlags::from_bits_retain(mask), on)
            .map_err(map_err)
    }

    fn set_color_operation(&self, op: u32) -> PyResult<()> {
        CoreTObj::from_struct(self.inner.clone())
            .set_color_operation(ColorMap::from(op))
            .map_err(map_err)
    }

    fn set_alpha_operation(&self, op: u32) -> PyResult<()> {
        CoreTObj::from_struct(self.inner.clone())
            .set_alpha_operation(AlphaMap::from(op))
            .map_err(map_err)
    }

    // ----- blending / filter --------------------------------------
    #[getter]
    fn blending(&self) -> PyResult<f32> {
        CoreTObj::from_struct(self.inner.clone()).blending().map_err(map_err)
    }

    #[setter]
    fn set_blending(&self, v: f32) -> PyResult<()> {
        CoreTObj::from_struct(self.inner.clone()).set_blending(v).map_err(map_err)
    }

    #[getter]
    fn mag_filter(&self) -> PyResult<u32> {
        CoreTObj::from_struct(self.inner.clone())
            .mag_filter()
            .map(u32::from)
            .map_err(map_err)
    }

    #[setter]
    fn set_mag_filter(&self, v: u32) -> PyResult<()> {
        CoreTObj::from_struct(self.inner.clone())
            .set_mag_filter(GxTexFilter::from(v))
            .map_err(map_err)
    }

    // ----- image / tlut refs --------------------------------------
    /// Attached `Image`, or `None`.
    #[getter]
    fn image_data(&self) -> Option<PyImage> {
        CoreTObj::from_struct(self.inner.clone())
            .image_data()
            .map(|i| PyImage { inner: i.0 })
    }

    /// Set / clear the `Image` reference.  Pass `None` to detach.
    #[pyo3(signature = (img=None))]
    fn set_image_data(&self, img: Option<&PyImage>) {
        CoreTObj::from_struct(self.inner.clone()).set_image_data(
            img.map(|i| CoreImage::from_struct(i.inner.clone())),
        );
    }

    /// Attached `Tlut` palette struct, or `None`.  Returned as raw
    /// `HsdStruct` since this binding doesn't ship a typed Tlut wrapper
    /// (paletted formats aren't on the H2/H3 scope — see
    /// `docs/roadmap.md` § texture re-pack).
    #[getter]
    fn tlut_data(&self) -> Option<PyHsdStruct> {
        self.inner
            .borrow()
            .get_reference(0x50)
            .map(|s| PyHsdStruct { inner: s })
    }

    #[pyo3(signature = (tlut=None))]
    fn set_tlut_data(&self, tlut: Option<&PyHsdStruct>) {
        let mut s = self.inner.borrow_mut();
        s.set_reference(0x50, tlut.map(|t| t.inner.clone()));
    }

    /// Attached `Lod` (HSD_TOBJ_LOD) struct, or `None`.  When NULL,
    /// GX hardware applies global default min_filter / lod_bias /
    /// aniso — see [`PyLod`] for the effect of an explicit attachment.
    #[getter]
    fn lod_data(&self) -> Option<PyLod> {
        CoreTObj::from_struct(self.inner.clone())
            .lod_data()
            .map(|l| PyLod { inner: l.0 })
    }

    /// Attach (or detach) the `Lod` reference at offset 0x54.  Pass
    /// `None` to detach.
    #[pyo3(signature = (lod=None))]
    fn set_lod_data(&self, lod: Option<&PyLod>) {
        CoreTObj::from_struct(self.inner.clone()).set_lod_data(
            lod.map(|l| CoreLod::from_struct(l.inner.clone())),
        );
    }

    /// Convenience: build a fresh `Lod` with the supplied fields and
    /// attach it.  Mirrors the HSDLib `HSD_TOBJ_LOD { … }` initializer
    /// pattern in one call — useful when you just want a non-NULL LOD
    /// with deterministic min_filter / aniso to overwrite the GX
    /// hardware default sampler behaviour.  Field defaults (when
    /// omitted) follow `Lod.alloc()`: `min_filter=GX_NEAR (0)`,
    /// `bias=0.0`, `bias_clamp=False`, `enable_edge_lod=False`,
    /// `anisotropy=GX_ANISO_1 (0)`.  All ints are raw GX enum values
    /// (see HSDLib `GXTexFilter` / `GXAnisotropy`).
    #[pyo3(signature = (
        min_filter = 0,
        bias = 0.0,
        bias_clamp = false,
        enable_edge_lod = false,
        anisotropy = 0,
    ))]
    fn set_lod(
        &self,
        min_filter: u32,
        bias: f32,
        bias_clamp: bool,
        enable_edge_lod: bool,
        anisotropy: u32,
    ) -> PyResult<()> {
        let lod = CoreLod::allocate_default();
        lod.set_min_filter(GxTexFilter::from(min_filter))
            .map_err(map_err)?;
        lod.set_bias(bias).map_err(map_err)?;
        lod.set_bias_clamp(bias_clamp).map_err(map_err)?;
        lod.set_enable_edge_lod(enable_edge_lod).map_err(map_err)?;
        lod.set_anisotropy(GxAnisotropy::from(anisotropy))
            .map_err(map_err)?;
        CoreTObj::from_struct(self.inner.clone()).set_lod_data(Some(lod));
        Ok(())
    }

    /// Decoded snapshot of every TObj field as a Python dict.  Mirrors
    /// the HSDLib `HSD_TOBJ` accessor surface: raw flags + decoded
    /// nibbles (`coord_type` / `color_op` / `alpha_op`), per-bit
    /// lightmap booleans, plus presence booleans for the 4 child refs
    /// (image / tlut / lod / tev).  Identity is reported as `addr`
    /// (the underlying struct's Rc pointer cast to int).  Numeric
    /// fields use raw GX enum integers — the Python caller decodes
    /// against HSDLib's own enum definitions.
    fn to_dict<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let v = CoreTObj::from_struct(self.inner.clone());
        let d = PyDict::new(py);
        d.set_item("addr", Rc::as_ptr(&self.inner) as usize)?;
        d.set_item("next_present", v.next().is_some())?;
        d.set_item("tex_map_id", u32::from(v.tex_map_id().map_err(map_err)?))?;
        d.set_item(
            "tex_gen_src",
            u32::from(v.tex_gen_src().map_err(map_err)?),
        )?;
        d.set_item(
            "rotation",
            (
                v.rx().map_err(map_err)?,
                v.ry().map_err(map_err)?,
                v.rz().map_err(map_err)?,
            ),
        )?;
        d.set_item(
            "scale",
            (
                v.sx().map_err(map_err)?,
                v.sy().map_err(map_err)?,
                v.sz().map_err(map_err)?,
            ),
        )?;
        d.set_item(
            "translation",
            (
                v.tx().map_err(map_err)?,
                v.ty().map_err(map_err)?,
                v.tz().map_err(map_err)?,
            ),
        )?;
        d.set_item("wrap_s", u32::from(v.wrap_s().map_err(map_err)?))?;
        d.set_item("wrap_t", u32::from(v.wrap_t().map_err(map_err)?))?;
        d.set_item("repeat_s", v.repeat_s().map_err(map_err)?)?;
        d.set_item("repeat_t", v.repeat_t().map_err(map_err)?)?;
        let flags = v.flags().map_err(map_err)?;
        d.set_item("flags", flags.bits())?;
        d.set_item(
            "coord_type",
            u32::from(v.coord_type().map_err(map_err)?),
        )?;
        d.set_item(
            "color_op",
            u32::from(v.color_operation().map_err(map_err)?),
        )?;
        d.set_item(
            "alpha_op",
            u32::from(v.alpha_operation().map_err(map_err)?),
        )?;
        d.set_item("is_lightmap_diffuse", v.is_lightmap_diffuse().map_err(map_err)?)?;
        d.set_item("is_lightmap_specular", v.is_lightmap_specular().map_err(map_err)?)?;
        d.set_item("is_lightmap_ambient", v.is_lightmap_ambient().map_err(map_err)?)?;
        d.set_item("is_lightmap_ext", v.is_lightmap_ext().map_err(map_err)?)?;
        d.set_item("is_lightmap_shadow", v.is_lightmap_shadow().map_err(map_err)?)?;
        d.set_item("is_bump", v.is_bump().map_err(map_err)?)?;
        d.set_item("blending", v.blending().map_err(map_err)?)?;
        d.set_item(
            "mag_filter",
            u32::from(v.mag_filter().map_err(map_err)?),
        )?;
        d.set_item("image_data_present", v.image_data().is_some())?;
        d.set_item("tlut_data_present", v.tlut_data().is_some())?;
        d.set_item("lod_data_present", v.lod_data().is_some())?;
        d.set_item("tev_data_present", v.tev_data().is_some())?;
        Ok(d)
    }

    fn __eq__(&self, other: &Self) -> bool {
        ptr_eq(&self.inner, &other.inner)
    }

    fn __hash__(&self) -> isize {
        Rc::as_ptr(&self.inner) as isize
    }

    fn __repr__(&self) -> String {
        format!(
            "<hsdraw.TObj addr=0x{:X}>",
            Rc::as_ptr(&self.inner) as usize
        )
    }
}

// =====================================================================
// Lod typed view  (HSDLib HSD_TOBJ_LOD accessor)
// =====================================================================

/// Typed view onto a 0x10-byte HSD_TOBJ_LOD struct.  Per-TObj
/// min-filter / LOD-bias / anisotropy settings.  When a TObj has no
/// LOD attached, GX hardware uses the global default sampler — for
/// textures expecting an explicit LOD this can produce a footprint-
/// averaging look on minified texels.  Construct via `Lod.alloc()`,
/// configure with the per-field setters, and attach via
/// `TObj.set_lod_data(lod)` (or build + attach in one step using the
/// `TObj.set_lod(...)` convenience).
///
/// All enum-valued fields use the raw GX enum integer (not a Python
/// enum class); see HSDLib `GXTexFilter` and `GXAnisotropy` for the
/// numeric mapping.
#[pyclass(name = "Lod", module = "hsdraw", unsendable)]
struct PyLod {
    inner: StructRef,
}

#[pymethods]
impl PyLod {
    /// Allocate a fresh 0x10-byte HSD_TOBJ_LOD.  All fields zero —
    /// `min_filter = GX_NEAR (0)`, `bias = 0.0`,
    /// `bias_clamp = False`, `enable_edge_lod = False`,
    /// `anisotropy = GX_ANISO_1 (0)`.
    #[staticmethod]
    fn alloc() -> Self {
        Self { inner: CoreLod::allocate_default().0 }
    }

    #[staticmethod]
    fn from_struct(s: &PyHsdStruct) -> Self {
        Self { inner: s.inner.clone() }
    }

    fn as_struct(&self) -> PyHsdStruct {
        PyHsdStruct { inner: self.inner.clone() }
    }

    #[getter]
    fn min_filter(&self) -> PyResult<u32> {
        CoreLod::from_struct(self.inner.clone())
            .min_filter()
            .map(u32::from)
            .map_err(map_err)
    }

    #[setter]
    fn set_min_filter(&self, v: u32) -> PyResult<()> {
        CoreLod::from_struct(self.inner.clone())
            .set_min_filter(GxTexFilter::from(v))
            .map_err(map_err)
    }

    #[getter]
    fn bias(&self) -> PyResult<f32> {
        CoreLod::from_struct(self.inner.clone())
            .bias()
            .map_err(map_err)
    }

    #[setter]
    fn set_bias(&self, v: f32) -> PyResult<()> {
        CoreLod::from_struct(self.inner.clone())
            .set_bias(v)
            .map_err(map_err)
    }

    #[getter]
    fn bias_clamp(&self) -> PyResult<bool> {
        CoreLod::from_struct(self.inner.clone())
            .bias_clamp()
            .map_err(map_err)
    }

    #[setter]
    fn set_bias_clamp(&self, v: bool) -> PyResult<()> {
        CoreLod::from_struct(self.inner.clone())
            .set_bias_clamp(v)
            .map_err(map_err)
    }

    #[getter]
    fn enable_edge_lod(&self) -> PyResult<bool> {
        CoreLod::from_struct(self.inner.clone())
            .enable_edge_lod()
            .map_err(map_err)
    }

    #[setter]
    fn set_enable_edge_lod(&self, v: bool) -> PyResult<()> {
        CoreLod::from_struct(self.inner.clone())
            .set_enable_edge_lod(v)
            .map_err(map_err)
    }

    #[getter]
    fn anisotropy(&self) -> PyResult<u32> {
        CoreLod::from_struct(self.inner.clone())
            .anisotropy()
            .map(u32::from)
            .map_err(map_err)
    }

    #[setter]
    fn set_anisotropy(&self, v: u32) -> PyResult<()> {
        CoreLod::from_struct(self.inner.clone())
            .set_anisotropy(GxAnisotropy::from(v))
            .map_err(map_err)
    }

    /// Decoded snapshot of every Lod field as a Python dict.  Mirrors
    /// the HSD_TOBJ_LOD layout (TrimmedSize 0x10): all five fields plus
    /// `addr` (Rc identity).  Numeric fields are raw GX enum integers.
    fn to_dict<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let v = CoreLod::from_struct(self.inner.clone());
        let d = PyDict::new(py);
        d.set_item("addr", Rc::as_ptr(&self.inner) as usize)?;
        d.set_item(
            "min_filter",
            u32::from(v.min_filter().map_err(map_err)?),
        )?;
        d.set_item("bias", v.bias().map_err(map_err)?)?;
        d.set_item("bias_clamp", v.bias_clamp().map_err(map_err)?)?;
        d.set_item(
            "enable_edge_lod",
            v.enable_edge_lod().map_err(map_err)?,
        )?;
        d.set_item(
            "anisotropy",
            u32::from(v.anisotropy().map_err(map_err)?),
        )?;
        Ok(d)
    }

    fn __eq__(&self, other: &Self) -> bool {
        ptr_eq(&self.inner, &other.inner)
    }

    fn __hash__(&self) -> isize {
        Rc::as_ptr(&self.inner) as isize
    }

    fn __repr__(&self) -> String {
        format!(
            "<hsdraw.Lod addr=0x{:X}>",
            Rc::as_ptr(&self.inner) as usize
        )
    }
}

// =====================================================================
// Image typed view  (HSDLib HSD_Image accessor)
// =====================================================================

/// Typed view onto a 0x18-byte HSD_Image struct.  Holds a reference to
/// the raw GX-encoded byte payload (offset 0) plus width / height /
/// format / mipmap / min_lod / max_lod fields.  Construct via
/// `Image.alloc()`, attach raw bytes via `set_image_data_bytes(b)`, and
/// wire into a TObj via `TObj.set_image_data(img)`.
#[pyclass(name = "Image", module = "hsdraw", unsendable)]
struct PyImage {
    inner: StructRef,
}

#[pymethods]
impl PyImage {
    /// Allocate a fresh 0x18-byte HSD_Image.  All fields zero.
    #[staticmethod]
    fn alloc() -> Self {
        Self { inner: CoreImage::allocate_default().0 }
    }

    #[staticmethod]
    fn from_struct(s: &PyHsdStruct) -> Self {
        Self { inner: s.inner.clone() }
    }

    fn as_struct(&self) -> PyHsdStruct {
        PyHsdStruct { inner: self.inner.clone() }
    }

    /// Raw GX-encoded bytes (already-encoded — use
    /// `hsdraw.gx_encode(format, w, h, rgba)` to produce these from
    /// a 4-channel RGBA8 source).  `None` if no payload is attached.
    fn image_data<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
        CoreImage::from_struct(self.inner.clone())
            .image_data()
            .map(|v| PyBytes::new(py, &v))
    }

    /// Wrap raw GX-encoded bytes in a fresh leaf buffer struct and
    /// attach at offset 0.  Marks the buffer as 0x20-aligned (HSDLib
    /// `IsBufferAligned` convention for textures).
    fn set_image_data_bytes(&self, bytes: &Bound<'_, PyBytes>) {
        CoreImage::from_struct(self.inner.clone())
            .set_image_data_bytes(bytes.as_bytes().to_vec());
    }

    #[getter]
    fn width(&self) -> PyResult<i16> {
        CoreImage::from_struct(self.inner.clone()).width().map_err(map_err)
    }

    #[setter]
    fn set_width(&self, v: i16) -> PyResult<()> {
        CoreImage::from_struct(self.inner.clone()).set_width(v).map_err(map_err)
    }

    #[getter]
    fn height(&self) -> PyResult<i16> {
        CoreImage::from_struct(self.inner.clone()).height().map_err(map_err)
    }

    #[setter]
    fn set_height(&self, v: i16) -> PyResult<()> {
        CoreImage::from_struct(self.inner.clone()).set_height(v).map_err(map_err)
    }

    /// `GX_TF_*` enum value as `u32` (0=I4, 1=I8, 2=IA4, 3=IA8,
    /// 4=RGB565, 5=RGB5A3, 6=RGBA8, 14=CMP).
    #[getter]
    fn format(&self) -> PyResult<u32> {
        CoreImage::from_struct(self.inner.clone())
            .format()
            .map(u32::from)
            .map_err(map_err)
    }

    #[setter]
    fn set_format(&self, v: u32) -> PyResult<()> {
        CoreImage::from_struct(self.inner.clone())
            .set_format(GxTexFmt::from(v))
            .map_err(map_err)
    }

    #[getter]
    fn mipmap(&self) -> PyResult<i32> {
        CoreImage::from_struct(self.inner.clone()).mipmap().map_err(map_err)
    }

    #[setter]
    fn set_mipmap(&self, v: i32) -> PyResult<()> {
        CoreImage::from_struct(self.inner.clone()).set_mipmap(v).map_err(map_err)
    }

    #[getter]
    fn min_lod(&self) -> PyResult<f32> {
        CoreImage::from_struct(self.inner.clone()).min_lod().map_err(map_err)
    }

    #[setter]
    fn set_min_lod(&self, v: f32) -> PyResult<()> {
        CoreImage::from_struct(self.inner.clone()).set_min_lod(v).map_err(map_err)
    }

    #[getter]
    fn max_lod(&self) -> PyResult<f32> {
        CoreImage::from_struct(self.inner.clone()).max_lod().map_err(map_err)
    }

    #[setter]
    fn set_max_lod(&self, v: f32) -> PyResult<()> {
        CoreImage::from_struct(self.inner.clone()).set_max_lod(v).map_err(map_err)
    }

    fn __eq__(&self, other: &Self) -> bool {
        ptr_eq(&self.inner, &other.inner)
    }

    fn __hash__(&self) -> isize {
        Rc::as_ptr(&self.inner) as isize
    }

    fn __repr__(&self) -> String {
        format!(
            "<hsdraw.Image addr=0x{:X}>",
            Rc::as_ptr(&self.inner) as usize
        )
    }
}

// =====================================================================
// MeshBuilder  (HSDLib POBJ_Generator equivalent for Phase 1)
// =====================================================================

/// Phase 1 POBJ writer.  Push positions / normals / colors / UVs /
/// triangles, then call `build()` for a `Pobj`.  See
/// `docs/python_api.md` for the full surface; the constraints in one
/// line: TRIANGLES only, ≤ 65,535 verts per POBJ, fixed attribute
/// formats (POS F32×3, NRM F32×3, CLR0 RGBA8, TEX0 F32×2), no envelope
/// rigging.
#[pyclass(name = "MeshBuilder", module = "hsdraw", unsendable)]
struct PyMeshBuilder {
    inner: RefCell<CoreMeshBuilder>,
}

#[pymethods]
impl PyMeshBuilder {
    #[new]
    fn new() -> Self {
        Self { inner: RefCell::new(CoreMeshBuilder::new()) }
    }

    /// Bulk-load mesh data from flat per-component sequences in a
    /// single PyO3 call.  Equivalent to a `MeshBuilder()` + N×
    /// `add_position` / `add_normal` / `add_color` / `add_uv` /
    /// `add_triangle` calls but saves the per-vertex Python→Rust
    /// transition cost.
    ///
    /// Argument layout (all keyword-only, validated lengths):
    ///   - `positions`: flat `[x0, y0, z0, x1, y1, z1, …]` float
    ///     sequence.  Length must be a multiple of 3; the count of
    ///     vertices is `len(positions) / 3`.
    ///   - `triangles`: flat `[i0, i1, i2, i0, i1, i2, …]` int
    ///     sequence.  Length must be a multiple of 3; each index
    ///     must be in `[0, n_verts)`.
    ///   - `normals` (optional): flat `[nx, ny, nz, …]` floats; must
    ///     be `3 * n_verts` long when supplied.
    ///   - `colors` (optional): flat `[r, g, b, a, …]` u8 sequence
    ///     (or `bytes`); must be `4 * n_verts` long.
    ///   - `uvs` (optional): flat `[u, v, …]` float sequence; must
    ///     be `2 * n_verts` long.
    ///
    /// All length validation runs before any push, so on error the
    /// returned builder is empty.  Per-vertex envelope rigging /
    /// PNMTXIDX bytes are not part of this bulk path — call
    /// `add_envelope` / `add_envelope_index` / `add_pos_mat_idx` on
    /// the returned builder afterwards.
    #[staticmethod]
    #[pyo3(signature = (
        *,
        positions,
        triangles,
        normals = None,
        colors = None,
        uvs = None,
    ))]
    fn from_arrays(
        positions: Vec<f32>,
        triangles: Vec<u32>,
        normals: Option<Vec<f32>>,
        colors: Option<Vec<u8>>,
        uvs: Option<Vec<f32>>,
    ) -> PyResult<Self> {
        if !positions.len().is_multiple_of(3) {
            return Err(PyValueError::new_err(
                "MeshBuilder.from_arrays: positions length must be divisible by 3 (got flat [x,y,z, …])",
            ));
        }
        let n_verts = positions.len() / 3;
        if !triangles.len().is_multiple_of(3) {
            return Err(PyValueError::new_err(
                "MeshBuilder.from_arrays: triangles length must be divisible by 3 (got flat [i,j,k, …])",
            ));
        }
        if let Some(ref nrm) = normals {
            if nrm.len() != n_verts * 3 {
                return Err(PyValueError::new_err(format!(
                    "MeshBuilder.from_arrays: normals length must be 3 * n_verts = {} (got {})",
                    n_verts * 3,
                    nrm.len()
                )));
            }
        }
        if let Some(ref clr) = colors {
            if clr.len() != n_verts * 4 {
                return Err(PyValueError::new_err(format!(
                    "MeshBuilder.from_arrays: colors length must be 4 * n_verts = {} (got {})",
                    n_verts * 4,
                    clr.len()
                )));
            }
        }
        if let Some(ref uv) = uvs {
            if uv.len() != n_verts * 2 {
                return Err(PyValueError::new_err(format!(
                    "MeshBuilder.from_arrays: uvs length must be 2 * n_verts = {} (got {})",
                    n_verts * 2,
                    uv.len()
                )));
            }
        }

        let mut mb = CoreMeshBuilder::new();
        for c in positions.chunks_exact(3) {
            mb.add_position(c[0], c[1], c[2]);
        }
        if let Some(nrm) = normals {
            for c in nrm.chunks_exact(3) {
                mb.add_normal(c[0], c[1], c[2]);
            }
        }
        if let Some(clr) = colors {
            for c in clr.chunks_exact(4) {
                mb.add_color(c[0], c[1], c[2], c[3]);
            }
        }
        if let Some(uv) = uvs {
            for c in uv.chunks_exact(2) {
                mb.add_uv(c[0], c[1]);
            }
        }
        for c in triangles.chunks_exact(3) {
            mb.add_triangle(c[0], c[1], c[2]);
        }
        Ok(Self { inner: RefCell::new(mb) })
    }

    fn add_position(&self, x: f32, y: f32, z: f32) {
        self.inner.borrow_mut().add_position(x, y, z);
    }

    fn add_normal(&self, x: f32, y: f32, z: f32) {
        self.inner.borrow_mut().add_normal(x, y, z);
    }

    fn add_color(&self, r: u8, g: u8, b: u8, a: u8) {
        self.inner.borrow_mut().add_color(r, g, b, a);
    }

    fn add_uv(&self, u: f32, v: f32) {
        self.inner.borrow_mut().add_uv(u, v);
    }

    fn add_triangle(&self, i0: u32, i1: u32, i2: u32) {
        self.inner.borrow_mut().add_triangle(i0, i1, i2);
    }

    /// **Deprecated** — emits `DeprecationWarning` and is now a no-op
    /// at the POBJ.flags level.  The historical 0x4000 bit lands inside
    /// HSDLib's `POBJ_TYPE_MASK` (0xE000) without matching any valid
    /// POBJ-type encoding, so renderers dispatching on the type nibble
    /// treat affected POBJs as unknown and drop texture-coord
    /// generation.  Set cull mode via `PeDesc` on the parent `MObj`
    /// instead.
    fn set_cull_back(&self, py: Python<'_>, on: bool) -> PyResult<()> {
        PyErr::warn(
            py,
            &py.get_type::<PyDeprecationWarning>(),
            c"MeshBuilder.set_cull_back is deprecated — POBJ.flags 0x4000 collides with POBJ_TYPE_MASK and is mis-handled by renderers dispatching on the POBJ type nibble.  Use PeDesc for cull mode.  This call is now a no-op.",
            1,
        )?;
        #[allow(deprecated)]
        self.inner.borrow_mut().set_cull_back(on);
        Ok(())
    }

    /// **Deprecated** — same trap as [`Self::set_cull_back`] for the
    /// 0x8000 bit (collides with `POBJ_FLAG.ENVELOPE`).  No-op at the
    /// POBJ.flags level; emits `DeprecationWarning`.
    fn set_cull_front(&self, py: Python<'_>, on: bool) -> PyResult<()> {
        PyErr::warn(
            py,
            &py.get_type::<PyDeprecationWarning>(),
            c"MeshBuilder.set_cull_front is deprecated — POBJ.flags 0x8000 collides with POBJ_FLAG.ENVELOPE.  Use PeDesc for cull mode.  This call is now a no-op.",
            1,
        )?;
        #[allow(deprecated)]
        self.inner.borrow_mut().set_cull_front(on);
        Ok(())
    }

    /// Toggle Phase 2 greedy `TRIANGLE_STRIP` decomposition.  On by
    /// default — pass `False` to force the Phase 1 single-`Triangles`-
    /// group emit path (handy when comparing DL bytecode against
    /// HSDLib's output, or when you need predictable byte layouts).
    fn set_use_triangle_strips(&self, on: bool) {
        self.inner.borrow_mut().set_use_triangle_strips(on);
    }

    /// Phase 3: add a new envelope (one matrix slot's worth of bone
    /// influences).  `weights` is a Python iterable of
    /// `(JObj, weight: float)` tuples; weights should sum to ~1.0.
    /// Returns the envelope index (0-based) for use with
    /// `add_envelope_index`.  Each envelope reserves 3 GX matrix
    /// slots (pos / normal / binormal), so the effective max is 85
    /// envelopes per POBJ — split into multiple POBJs above that.
    fn add_envelope(
        &self,
        weights: &Bound<'_, PyAny>,
    ) -> PyResult<u32> {
        // PyO3 0.28's `Vec<(PyJObj, f32)>` extraction needs
        // `FromPyObjectOwned`, which `#[pyclass]` types don't get for
        // free.  Iterate the input ourselves and downcast each tuple
        // element manually — same effect, fewer trait gymnastics.
        let mut core_weights: Vec<(CoreJObj, f32)> = Vec::new();
        for item in weights.try_iter()? {
            let pair = item?;
            let tup: (Bound<'_, PyAny>, f32) = pair.extract()?;
            let jobj_ref = tup.0.cast::<PyJObj>().map_err(|_| {
                PyTypeError::new_err(
                    "MeshBuilder.add_envelope: tuple element 0 must be a JObj",
                )
            })?;
            let jobj_inner = jobj_ref.borrow().inner.clone();
            core_weights.push((CoreJObj::from_struct(jobj_inner), tup.1));
        }
        Ok(self.inner.borrow_mut().add_envelope(core_weights))
    }

    /// Per-vertex envelope index (parallel to position pushes).  When
    /// envelopes are in use, every vertex must have an associated
    /// envelope; the count must match positions.  `env_idx` references
    /// the envelopes added via `add_envelope`.
    fn add_envelope_index(&self, env_idx: u32) {
        self.inner.borrow_mut().add_envelope_index(env_idx);
    }

    /// Toggle the no-envelope `GX_VA_PNMTXIDX` emission path (off by
    /// default).  When `True`, the builder will emit a DIRECT 1-byte
    /// `GX_VA_PNMTXIDX` attribute populated from per-vertex matrix
    /// indices pushed via `add_pos_mat_idx`, **without** setting
    /// `POBJ_FLAG.ENVELOPE` (= no envelope-pointer array attached at
    /// POBJ + 0x14).  Use this when your runtime expects every vertex
    /// to carry a matrix-index byte regardless of whether the mesh is
    /// skinned — the `Pobj.flags` u16 stays whatever you set on it
    /// separately (see `Pobj.flags` setter).
    ///
    /// Mutually exclusive with `add_envelope`: `build()` rejects the
    /// combination.  Toggling off (`False`) clears any indices already
    /// pushed via `add_pos_mat_idx`.
    fn set_use_pos_mat_idx(&self, on: bool) {
        self.inner.borrow_mut().set_use_pos_mat_idx(on);
    }

    /// Push one `GX_VA_PNMTXIDX` byte for the next vertex.  Implicitly
    /// activates the no-envelope PNMTXIDX path (= as if
    /// `set_use_pos_mat_idx(True)` had been called first).  When the
    /// path is active, the count of pushed indices must equal
    /// `positions` count at `build()` time.
    fn add_pos_mat_idx(&self, idx: u8) {
        self.inner.borrow_mut().add_pos_mat_idx(idx);
    }

    fn envelope_count(&self) -> usize {
        self.inner.borrow().envelope_count()
    }

    fn vertex_count(&self) -> usize {
        self.inner.borrow().vertex_count()
    }

    fn triangle_count(&self) -> usize {
        self.inner.borrow().triangle_count()
    }

    /// Validate inputs and produce the `Pobj`.  Consumes the builder
    /// (subsequent calls raise `RuntimeError`).
    fn build(&self) -> PyResult<PyPObj> {
        // Take the inner builder out so build's by-value consumption
        // works.  Subsequent calls find a default builder which fails
        // validation immediately — predictable error path.
        let mb = self.inner.replace(CoreMeshBuilder::new());
        let pobj = mb.build().map_err(map_err)?;
        Ok(PyPObj { inner: pobj.0 })
    }

    fn __repr__(&self) -> String {
        let mb = self.inner.borrow();
        format!(
            "<hsdraw.MeshBuilder verts={} tris={}>",
            mb.vertex_count(),
            mb.triangle_count()
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

/// Encode RGBA8 source bytes into a GX texture format payload.
/// Wraps `hsdraw_core::gx_image::encode_image_with_options`: pass
/// `format` as the `GxTexFmt` integer (4=RGB565, 5=RGB5A3, 6=RGBA8,
/// 14=CMP) and the raw RGBA8 source as `bytes` of exactly
/// `4 * width * height` bytes.  Output dimensions get padded to the
/// format's natural tile boundary (4 or 8), so the byte count matches
/// what `decode_image` consumes.  Other formats (I4/I8/IA4/IA8/CIxx)
/// raise `ValueError`.
///
/// `swap_rb_for_rgb5a3` (kwarg, default `False`): pre-swap R↔B in the
/// source RGBA before the RGB5A3 encode loop.  Use when the target
/// renderer's RGB5A3 sampler reads channels in BGR order; no effect
/// on RGBA8 / RGB565 / CMP.  See `EncodeOptions::swap_rb_for_rgb5a3`.
#[pyfunction]
#[pyo3(signature = (format, width, height, rgba, /, swap_rb_for_rgb5a3 = false))]
fn gx_encode<'py>(
    py: Python<'py>,
    format: u32,
    width: u32,
    height: u32,
    rgba: &Bound<'_, PyBytes>,
    swap_rb_for_rgb5a3: bool,
) -> PyResult<Bound<'py, PyBytes>> {
    let fmt = GxTexFmt::from(format);
    let opts = hsdraw_core::gx_image::EncodeOptions { swap_rb_for_rgb5a3 };
    let out = hsdraw_core::gx_image::encode_image_with_options(
        fmt,
        width,
        height,
        rgba.as_bytes(),
        opts,
    )
    .map_err(|e| PyValueError::new_err(format!("gx_encode: {:?}", e)))?;
    Ok(PyBytes::new(py, &out))
}

/// Decode a GX texture format payload back to RGBA8.
///
/// Wraps `hsdraw_core::gx_image::decode_image`: pass `format` as the
/// `GxTexFmt` integer (0=I4, 1=I8, 2=IA4, 3=IA8, 4=RGB565, 5=RGB5A3,
/// 6=RGBA8, 8=CI4, 9=CI8, 10=CI14X2, 14=CMP) and the raw GX-encoded
/// bytes via `gx_bytes` (length must be at least the format's
/// `image_size(w, h)`).  Output is exactly `4 * width * height` bytes
/// of RGBA8, byte order R, G, B, A — the Rust core already mirrors
/// HSDLib's BGRA→RGBA swap internally for RGBA8 / CMP, so callers
/// don't need to swap channels themselves (csx scripts on top of
/// HSDLib do, because HSDLib's `GetDecodedImageData()` returns BGRA).
///
/// Paletted formats (CI4 / CI8 / CI14X2) require the TLUT payload via
/// `palette` plus `palette_format` (the `GxTlutFmt` integer: 0=IA8,
/// 1=RGB565, 2=RGB5A3 — defaults to RGB5A3, the most common course-
/// data choice).  Non-paletted formats ignore both palette args.
///
/// Useful for pipelines that need to surface a TObj's
/// `Image.image_data()` to Blender / Pillow / etc. without going
/// through the csx + ImageSharp + dotnet-script chain.
#[pyfunction]
#[pyo3(signature = (format, width, height, gx_bytes, palette = None, palette_format = 2, /))]
fn gx_decode<'py>(
    py: Python<'py>,
    format: u32,
    width: u32,
    height: u32,
    gx_bytes: &Bound<'_, PyBytes>,
    palette: Option<&Bound<'_, PyBytes>>,
    palette_format: u32,
) -> PyResult<Bound<'py, PyBytes>> {
    let fmt = GxTexFmt::from(format);
    let pal_fmt = GxTlutFmt::from(palette_format);
    // Stash the palette borrow before constructing the tuple so the
    // borrow's lifetime out-lives the `pal_arg` we hand to decode_image.
    let pal_bytes = palette.map(|b| b.as_bytes());
    let pal_arg = pal_bytes.map(|data| (pal_fmt, data));
    let out = hsdraw_core::gx_image::decode_image(
        fmt,
        width,
        height,
        gx_bytes.as_bytes(),
        pal_arg,
    )
    .map_err(|e| PyValueError::new_err(format!("gx_decode: {:?}", e)))?;
    Ok(PyBytes::new(py, &out))
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

/// Accept any of the typed-view wrappers or a raw `HsdStruct` and
/// return the shared `StructRef`.  Anything else is `TypeError`.  Used
/// by every cross-typed-view setter (`Dat.add_root`, `DObj.set_mobj`,
/// etc.) so a Python user can pass whatever handle they happen to have.
fn struct_ref_from_any(any: &Bound<'_, PyAny>) -> PyResult<StructRef> {
    if let Ok(j) = any.cast::<PyJObj>() {
        return Ok(j.borrow().inner.clone());
    }
    if let Ok(d) = any.cast::<PyDObj>() {
        return Ok(d.borrow().inner.clone());
    }
    if let Ok(p) = any.cast::<PyPObj>() {
        return Ok(p.borrow().inner.clone());
    }
    if let Ok(m) = any.cast::<PyMObj>() {
        return Ok(m.borrow().inner.clone());
    }
    if let Ok(m) = any.cast::<PyMaterial>() {
        return Ok(m.borrow().inner.clone());
    }
    if let Ok(p) = any.cast::<PyPeDesc>() {
        return Ok(p.borrow().inner.clone());
    }
    if let Ok(s) = any.cast::<PySObj>() {
        return Ok(s.borrow().inner.clone());
    }
    if let Ok(d) = any.cast::<PyJObjDesc>() {
        return Ok(d.borrow().inner.clone());
    }
    if let Ok(t) = any.cast::<PyTObj>() {
        return Ok(t.borrow().inner.clone());
    }
    if let Ok(i) = any.cast::<PyImage>() {
        return Ok(i.borrow().inner.clone());
    }
    if let Ok(s) = any.cast::<PyHsdStruct>() {
        return Ok(s.borrow().inner.clone());
    }
    Err(PyTypeError::new_err(
        "expected JObj / DObj / Pobj / MObj / Material / PeDesc / SObj / JObjDesc / TObj / Image / HsdStruct",
    ))
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
    m.add_function(wrap_pyfunction!(gx_encode, m)?)?;
    m.add_function(wrap_pyfunction!(gx_decode, m)?)?;
    m.add_class::<PyDat>()?;
    m.add_class::<PyRoot>()?;
    m.add_class::<PyHsdStruct>()?;
    m.add_class::<PyJObj>()?;
    m.add_class::<PyDObj>()?;
    m.add_class::<PyPObj>()?;
    m.add_class::<PyMObj>()?;
    m.add_class::<PyMaterial>()?;
    m.add_class::<PyPeDesc>()?;
    m.add_class::<PySObj>()?;
    m.add_class::<PyJObjDesc>()?;
    m.add_class::<PyTObj>()?;
    m.add_class::<PyLod>()?;
    m.add_class::<PyImage>()?;
    m.add_class::<PyMeshBuilder>()?;
    Ok(())
}
