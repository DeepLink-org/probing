use pyo3::prelude::*;
use pyo3::types::PyModule;

use crate::extensions;
use crate::features::config;
use crate::features::tracing;
use crate::features::vm_tracer::{
    _get_python_frames, _get_python_stacks, disable_tracer, enable_tracer, initialize_globals,
};
use crate::pkg::TCPStore;
use probing_core::ENGINE;

#[pyfunction]
fn query_json(_py: Python, sql: String) -> PyResult<String> {
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

pub fn create_probing_module() -> PyResult<()> {
    if initialize_globals() {
        #[cfg(feature = "tracing")]
        Python::with_gil(|_| enable_tracer())?;
    }
    Python::with_gil(|py| -> PyResult<()> {
        let sys = PyModule::import(py, "sys")?;
        let modules = sys.getattr("modules")?;

        if !modules.contains("probing")? {
            let m = PyModule::new(py, "probing")?;
            modules.set_item("probing", m)?;
        }

        let m = PyModule::import(py, "probing")?;
        let already_initialized = m.hasattr(pyo3::intern!(py, "_C"))?;

        if !already_initialized {
            m.setattr(pyo3::intern!(py, "_C"), 42)?;
            m.add_class::<extensions::python::ExternalTable>()?;
            m.add_class::<TCPStore>()?;
            m.add_function(wrap_pyfunction!(query_json, py)?)?;
            m.add_function(wrap_pyfunction!(enable_tracer, py)?)?;
            m.add_function(wrap_pyfunction!(disable_tracer, py)?)?;
            m.add_function(wrap_pyfunction!(_get_python_stacks, py)?)?;
            m.add_function(wrap_pyfunction!(_get_python_frames, py)?)?;

            // Register config module
            config::register_config_module(&m)?;
        }

        // Always try to register _tracing module (internal Rust implementation)
        if !m.hasattr(pyo3::intern!(py, "_tracing"))? {
            let _tracing_module = PyModule::new(py, "_tracing")?;
            tracing::register_tracing_module(py, &_tracing_module)?;
            m.add_submodule(&_tracing_module)?;

            // Also add to sys.modules for direct import
            let sys = PyModule::import(py, "sys")?;
            let modules = sys.getattr("modules")?;
            modules.set_item("probing._tracing", &_tracing_module)?;
        }

        Ok(())
    })
}
