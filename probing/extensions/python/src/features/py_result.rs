use pyo3::exceptions::PyRuntimeError;
use pyo3::PyErr;

/// Map a displayable error into `PyRuntimeError` (Python API boundary).
pub fn runtime_err(err: impl std::fmt::Display) -> PyErr {
    PyRuntimeError::new_err(err.to_string())
}
