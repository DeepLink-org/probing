//! PyO3 entry for `probing.cli_main` (wheel `_core` — not L2 collector).

use pyo3::prelude::*;

use crate::cli_main as cli_main_impl;

#[pyfunction]
pub fn cli_main(py: Python, args: Vec<String>) -> PyResult<()> {
    // Skill install/update shells out to ``python -m probing.skills`` — use this interpreter.
    if let Ok(exe) = py.import("sys")?.getattr("executable")?.extract::<String>() {
        std::env::set_var("PROBING_PYTHON", exe);
    }
    if let Err(e) = cli_main_impl(args) {
        return Err(PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(
            e.to_string(),
        ));
    }
    Ok(())
}
