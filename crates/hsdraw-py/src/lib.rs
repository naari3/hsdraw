//! `hsdraw` Python module — PyO3 binding.  Phase 1 only exposes a `version()`
//! probe; the real `parse_dat` / `write_dat` surface lands in Phase 4 once the
//! reader is parity-verified against the csx golden.

use pyo3::prelude::*;

#[pyfunction]
fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[pymodule]
fn hsdraw(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(version, m)?)?;
    Ok(())
}
