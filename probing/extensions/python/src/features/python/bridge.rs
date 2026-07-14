//! FFI helpers shared by PyO3 bindings: thread bridge, Ele convert, errors.

use probing_core::{on_native_bridge_thread, run_on_native_thread};
use probing_proto::prelude::Ele;
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyFloat, PyInt, PyString};
use pyo3::PyErr;

/// Map a displayable error into `PyRuntimeError` (Python API boundary).
pub fn runtime_err(err: impl std::fmt::Display) -> PyErr {
    PyRuntimeError::new_err(err.to_string())
}

/// Run Rust/Python bridge work off the Python main thread and Tokio workers.
///
/// When already on the ``probing-native`` bridge worker (e.g. loading a Python
/// plugin from ``execute_python_code``), avoid ``py.detach`` — an extra thread
/// plus nested GIL handoffs can SIGABRT on Linux CI (pytest-cov + torch).
pub fn with_detached_native<R: Send + probing_core::runtime::BlockOnFallback + 'static>(
    f: impl FnOnce() -> R + Send + 'static,
) -> R {
    if on_native_bridge_thread() {
        return run_on_native_thread(f);
    }
    Python::attach(|py| py.detach(|| run_on_native_thread(f)))
}

/// Convert `Ele` to a Python object.
pub fn ele_to_python(py: Python, ele: &Ele) -> PyResult<Py<PyAny>> {
    let obj: Py<PyAny> = match ele {
        Ele::Nil => py.None(),
        Ele::BOOL(b) => PyBool::new(py, *b).to_owned().unbind().into(),
        Ele::I32(i) => PyInt::new(py, *i as i64).to_owned().unbind().into(),
        Ele::I64(i) => PyInt::new(py, *i).to_owned().unbind().into(),
        Ele::F32(f) => PyFloat::new(py, *f as f64).to_owned().unbind().into(),
        Ele::F64(f) => PyFloat::new(py, *f).to_owned().unbind().into(),
        Ele::Text(s) => PyString::new(py, s).to_owned().unbind().into(),
        Ele::Url(s) => PyString::new(py, s).to_owned().unbind().into(),
        Ele::DataTime(t) => {
            // Convert microsecond timestamp to string representation
            use std::time::{Duration, UNIX_EPOCH};
            let datetime = UNIX_EPOCH + Duration::from_micros(*t);
            let secs = datetime
                .duration_since(UNIX_EPOCH)
                .unwrap_or_else(|e| {
                    log::error!("DataTime before UNIX epoch ({t} µs): {e}; using 0");
                    Duration::ZERO
                })
                .as_secs()
                .to_string();
            PyString::new(py, &secs).to_owned().unbind().into()
        }
    };
    Ok(obj)
}

/// Convert a Python object to `Ele`.
pub fn python_to_ele(value: &Bound<'_, PyAny>) -> PyResult<Ele> {
    if value.is_none() {
        return Ok(Ele::Nil);
    }

    if let Ok(b) = value.extract::<bool>() {
        return Ok(Ele::BOOL(b));
    }

    if let Ok(i) = value.extract::<i64>() {
        if i >= i32::MIN as i64 && i <= i32::MAX as i64 {
            return Ok(Ele::I32(i as i32));
        }
        return Ok(Ele::I64(i));
    }

    if let Ok(f) = value.extract::<f64>() {
        return Ok(Ele::F64(f));
    }

    if let Ok(s) = value.extract::<String>() {
        return Ok(Ele::Text(s));
    }

    let s = value.str()?.to_string();
    Ok(Ele::Text(s))
}
