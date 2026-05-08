//! `hsdraw` Python module — PyO3 binding.
//!
//! Phase 4 surface: just enough to drive the Blender addon prototype.
//! - `version() -> str`
//! - `parse_dat(bytes) -> Dat`  (validates header & relocation table)
//! - `export_scene_json(bytes, source_dat=..., tex_dir=None) -> str`
//!   produces the same JSON shape as `mkgp2-patch/tools/hsd/
//!   hsd_export_for_blender.csx` (parity-verified by
//!   `crates/hsdraw-core/tests/parity.rs`).
//!
//! The fuller object-graph API hinted at in `docs/handoff.md`
//! (`dat.public_roots()`, `jobj.iter_descendants()`, …) is deferred to
//! Phase 5 — once the writer lands, alias semantics are nailed down, and
//! we can lock in the surface without expecting churn.

use std::path::PathBuf;

use hsdraw_core::{export, Dat as CoreDat};
use pyo3::exceptions::{PyIOError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyBytes;

#[pyfunction]
fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Lightweight handle around a parsed .dat.  Right now it only carries
/// the root names so callers can probe a file without committing to the
/// full export pipeline.  Full accessor surface is deferred to Phase 5.
#[pyclass(name = "Dat", module = "hsdraw")]
struct PyDat {
    // Stash the raw bytes so we can re-parse cheaply for export_scene without
    // forcing the user to re-supply them. .dat files are typically <10MB so
    // the memory hit is negligible compared to a re-upload from Python.
    raw: Vec<u8>,
    root_names: Vec<String>,
}

#[pymethods]
impl PyDat {
    /// Names of every root in declaration order (public + alias).
    fn root_names(&self) -> Vec<String> {
        self.root_names.clone()
    }

    /// Total file size in bytes (the raw input we were parsed from).
    fn byte_size(&self) -> usize {
        self.raw.len()
    }

    fn __repr__(&self) -> String {
        format!(
            "<hsdraw.Dat byte_size={} roots={}>",
            self.raw.len(),
            self.root_names.len()
        )
    }
}

/// Parse a .dat from raw bytes.  Returns a `Dat` handle on success.
#[pyfunction]
#[pyo3(signature = (data, /))]
fn parse_dat(data: &Bound<'_, PyBytes>) -> PyResult<PyDat> {
    let bytes = data.as_bytes().to_vec();
    let parsed = CoreDat::parse(&bytes)
        .map_err(|e| PyValueError::new_err(format!("parse_dat: {:?}", e)))?;
    let root_names = parsed.roots.iter().map(|r| r.name.clone()).collect();
    Ok(PyDat { raw: bytes, root_names })
}

/// Export the scene as a JSON string mirroring the csx golden.
///
/// `data` is the raw .dat bytes.  `source_dat` is the filename string
/// embedded into the output (matches csx's `--source-dat` flag).  If
/// `tex_dir` is provided, all referenced textures are decoded and dumped
/// as PNGs into that directory; otherwise no textures are written and
/// the JSON's `textures[*].file` paths are still populated for the
/// caller to resolve as they see fit.
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

/// Round-trip parse + write.  Returns freshly-serialized .dat bytes.
///
/// `optimize` (default True): drop unreachable structs and dedup byte-equal
/// buffer payloads.  `buffer_align` (default True): 0x20-align structs the
/// writer marks as buffers.  Disable both for byte-faithful debugging.
#[pyfunction]
#[pyo3(signature = (data, /, optimize=true, buffer_align=true))]
fn write_dat<'py>(
    py: Python<'py>,
    data: &Bound<'_, PyBytes>,
    optimize: bool,
    buffer_align: bool,
) -> PyResult<Bound<'py, PyBytes>> {
    use hsdraw_core::writer::WriteOptions;
    let bytes = data.as_bytes();
    let parsed = CoreDat::parse(bytes)
        .map_err(|e| PyValueError::new_err(format!("parse_dat: {:?}", e)))?;
    let opts = WriteOptions { optimize, buffer_align, trim: false };
    let out = parsed
        .write_with_options(opts)
        .map_err(|e| PyValueError::new_err(format!("write_dat: {:?}", e)))?;
    Ok(PyBytes::new(py, &out))
}

#[pymodule]
fn hsdraw(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(version, m)?)?;
    m.add_function(wrap_pyfunction!(parse_dat, m)?)?;
    m.add_function(wrap_pyfunction!(export_scene_json, m)?)?;
    m.add_function(wrap_pyfunction!(write_dat, m)?)?;
    m.add_class::<PyDat>()?;
    Ok(())
}
