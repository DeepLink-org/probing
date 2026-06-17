use pyo3::prelude::*;

use probing_cli::cli_main as cli_main_impl;
use probing_core::runtime::block_on;
use probing_core::ENGINE;

use crate::features::native_bridge::with_detached_native;
use crate::features::stack_tracer::{SignalTracer, StackTracer};
use crate::repl::PythonRepl;

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
        let result = block_on(async move {
            ENGINE
                .read()
                .await
                .async_query(sql.as_str())
                .await
        })
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()));

        let final_result = result.unwrap_or_default();
        serde_json::to_string(&final_result)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
    })
}

/// HTTP `GET /apis/pythonext/callstack` backend.
#[pyfunction]
#[pyo3(signature = (tid=None))]
pub fn api_callstack(tid: Option<i32>) -> PyResult<String> {
    let tid = tid.filter(|&t| t != 0);
    let frames = SignalTracer
        .trace(tid)
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;
    serde_json::to_string(&frames)
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
}

/// HTTP `POST /apis/pythonext/eval` backend.
#[pyfunction]
pub fn api_eval(code: &str) -> PyResult<String> {
    log::debug!("Python eval code: {code}");
    let mut repl = PythonRepl::default();
    let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| repl.process(code)));
    match out {
        Ok(Some(s)) => Ok(s),
        Ok(None) => Ok(String::new()),
        Err(_) => Ok(serde_json::json!({"error": "REPL execution panicked"}).to_string()),
    }
}

#[pyfunction]
pub fn cli_main(_py: Python, args: Vec<String>) -> PyResult<()> {
    if let Err(e) = cli_main_impl(args) {
        return Err(PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(
            e.to_string(),
        ));
    }
    Ok(())
}
