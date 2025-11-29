use pyo3::prelude::*;

use probing_core::ENGINE;
use probing_cli::cli_main as cli_main_impl;

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
    // Check if we're already inside a tokio runtime
    let result = match tokio::runtime::Handle::try_current() {
        Ok(_handle) => {
            // We're inside a runtime, spawn a new thread to avoid nested runtime error
            std::thread::spawn(move || {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap_or_else(|e| panic!("Failed to create current-thread runtime: {e}"))
                    .block_on(async { ENGINE.read().await.async_query(sql.as_str()).await })
            })
            .join()
            .map_err(|_| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>("Thread panicked"))?
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
        }
        Err(_) => {
            // Not in a runtime, create a new one
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(4)
                .enable_all()
                .build()
                .unwrap_or_else(|e| panic!("Failed to create multi-thread runtime: {e}"))
                .block_on(async { ENGINE.read().await.async_query(sql.as_str()).await })
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
        }
    };

    let final_result = result.unwrap_or_default();
    serde_json::to_string(&final_result)
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
}

#[pyfunction]
pub fn cli_main(_py: Python, args: Vec<String>) -> PyResult<()> {
    if let Err(e) = cli_main_impl(args) {
        return Err(PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()));
    }
    Ok(())
}

