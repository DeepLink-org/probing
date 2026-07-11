use std::panic::{catch_unwind, AssertUnwindSafe};

use pyo3::prelude::*;

use probing_cli::cli_main as cli_main_impl;
use probing_core::runtime::block_on;
use probing_core::ENGINE;

use crate::features::native_bridge::with_detached_native;
use crate::features::py_result::runtime_err;
use crate::features::stack_tracer::{is_backtrace_busy, SignalTracer, StackTracer};
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
        let df = block_on(async move { ENGINE.read().await.async_query(sql.as_str()).await })
            .map_err(runtime_err)?
            .map_err(runtime_err)?
            .ok_or_else(|| {
                pyo3::exceptions::PyRuntimeError::new_err(
                    "query returned no dataframe (engine unavailable or empty response)",
                )
            })?;
        serde_json::to_string(&df).map_err(runtime_err)
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
