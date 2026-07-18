//! PyO3 functions registered on the `probing._core` module
//! (config, SQL query, callstack, eval, enable flags).

use std::panic::{catch_unwind, AssertUnwindSafe};

use probing_core::config;
use probing_core::runtime::block_on;
use probing_core::ENGINE;
use pyo3::prelude::*;
use pyo3::types::PyModule;

use crate::features::python::bridge::{
    ele_to_python, python_to_ele, runtime_err, with_detached_native,
};
use crate::features::stacktrace::{is_backtrace_busy, SignalTracer, StackTracer};
use crate::repl::ReplSession;

fn callstack_error_json(message: &str) -> String {
    serde_json::json!({
        "error": message,
        "frames": [],
    })
    .to_string()
}

#[pyfunction]
pub fn should_enable_probing() -> bool {
    crate::python::should_enable_probing()
}

#[pyfunction]
pub fn is_enabled() -> bool {
    crate::python::is_enabled()
}

#[pyfunction]
pub fn query_json(_py: Python, sql: String) -> PyResult<String> {
    with_detached_native(move || {
        let bridge = block_on(async move { ENGINE.read().await.async_query(sql.as_str()).await })
            .map_err(|e| runtime_err(format!("probing runtime unavailable: {e}")))?;
        match bridge {
            Ok(Some(df)) => serde_json::to_string(&df).map_err(runtime_err),
            Ok(None) => Err(pyo3::exceptions::PyRuntimeError::new_err(
                "query returned nil (no tabular result; e.g. SET or non-SELECT)",
            )),
            Err(e) => Err(runtime_err(format!("engine SQL failed: {e}"))),
        }
    })
}

/// HTTP `GET /apis/pythonext/callstack` backend.
///
/// Always returns JSON (never raises into Python) so concurrent stack capture
/// cannot take down the training process via PyO3 exception propagation.
#[pyfunction]
#[pyo3(signature = (tid=None))]
pub fn api_callstack(tid: Option<i32>) -> PyResult<String> {
    let payload = catch_unwind(AssertUnwindSafe(|| {
        let tid = tid.filter(|&t| t != 0);
        match SignalTracer.trace(tid) {
            Ok(frames) => match serde_json::to_string(&frames) {
                Ok(json) => json,
                Err(e) => callstack_error_json(&format!("failed to encode callstack: {e}")),
            },
            Err(e) if is_backtrace_busy(&e) => callstack_error_json("callstack capture busy"),
            Err(e) => callstack_error_json(&e.to_string()),
        }
    }));
    Ok(match payload {
        Ok(json) => json,
        Err(_) => callstack_error_json("callstack capture panicked"),
    })
}

/// HTTP `POST /apis/pythonext/eval` backend.
#[pyfunction]
pub fn api_eval(code: &str) -> PyResult<String> {
    log::debug!("Python eval code: {code}");
    let mut session = ReplSession::new();
    let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| session.eval_code(code)));
    match out {
        Ok(s) => Ok(s),
        Err(_) => Ok(serde_json::json!({"error": "REPL execution panicked"}).to_string()),
    }
}

/// Get a configuration value.
///
/// Returns None if the key doesn't exist, otherwise returns the value
/// converted to the appropriate Python type.
#[pyfunction(name = "config_get")]
fn get(_py: Python, key: String) -> PyResult<Option<Py<PyAny>>> {
    with_detached_native(move || {
        let ele = block_on(async move { config::get(&key).await }).map_err(runtime_err)?;
        Python::attach(|py| match ele {
            Some(val) => Ok(Some(ele_to_python(py, &val)?)),
            None => Ok(None),
        })
    })
}

/// Set a configuration option through the engine extension system (starts servers, etc.).
#[pyfunction(name = "config_write")]
fn write(_py: Python, key: String, value: String) -> PyResult<()> {
    with_detached_native(move || {
        block_on(async move { config::write(&key, &value).await })
            .map_err(runtime_err)?
            .map_err(runtime_err)
    })
}

/// Set a configuration value.
///
/// Supports str, int, float, bool, and None values.
#[pyfunction(name = "config_set")]
fn set(_py: Python, key: String, value: Bound<'_, PyAny>) -> PyResult<()> {
    let value = value.unbind();
    with_detached_native(move || {
        let ele = Python::attach(|py| python_to_ele(value.bind(py)))?;
        block_on(async move { config::set(&key, ele).await }).map_err(runtime_err)
    })
}

/// Get a configuration value as string.
///
/// Returns None if the key doesn't exist, otherwise returns the value
/// converted to string.
#[pyfunction(name = "config_get_str")]
fn get_str(_py: Python, key: String) -> PyResult<Option<String>> {
    with_detached_native(move || {
        block_on(async move { config::get_str(&key).await }).map_err(runtime_err)
    })
}

/// Check if a configuration key exists.
#[pyfunction(name = "config_contains_key")]
fn contains_key(_py: Python, key: String) -> PyResult<bool> {
    with_detached_native(move || {
        block_on(async move { config::contains_key(&key).await }).map_err(runtime_err)
    })
}

/// Remove a configuration key and return its value.
#[pyfunction(name = "config_remove")]
fn remove(_py: Python, key: String) -> PyResult<Option<Py<PyAny>>> {
    with_detached_native(move || {
        let ele = block_on(async move { config::remove(&key).await }).map_err(runtime_err)?;
        Python::attach(|py| match ele {
            Some(val) => Ok(Some(ele_to_python(py, &val)?)),
            None => Ok(None),
        })
    })
}

/// Get all configuration keys.
#[pyfunction(name = "config_keys")]
fn keys(_py: Python) -> PyResult<Vec<String>> {
    with_detached_native(|| block_on(config::keys()).map_err(runtime_err))
}

/// Clear all configuration.
#[pyfunction(name = "config_clear")]
fn clear(_py: Python) -> PyResult<()> {
    with_detached_native(|| block_on(config::clear()).map_err(runtime_err))
}

/// Get the number of configuration entries.
#[pyfunction(name = "config_len")]
fn len(_py: Python) -> PyResult<usize> {
    with_detached_native(|| block_on(config::len()).map_err(runtime_err))
}

/// Check if the configuration store is empty.
#[pyfunction(name = "config_is_empty")]
fn is_empty(_py: Python) -> PyResult<bool> {
    with_detached_native(|| block_on(config::is_empty()).map_err(runtime_err))
}

/// Register the config functions directly to the probing Python module.
pub fn register_config_functions(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(get, module)?)?;
    module.add_function(wrap_pyfunction!(set, module)?)?;
    module.add_function(wrap_pyfunction!(write, module)?)?;
    module.add_function(wrap_pyfunction!(get_str, module)?)?;
    module.add_function(wrap_pyfunction!(contains_key, module)?)?;
    module.add_function(wrap_pyfunction!(remove, module)?)?;
    module.add_function(wrap_pyfunction!(keys, module)?)?;
    module.add_function(wrap_pyfunction!(clear, module)?)?;
    module.add_function(wrap_pyfunction!(len, module)?)?;
    module.add_function(wrap_pyfunction!(is_empty, module)?)?;

    Ok(())
}
