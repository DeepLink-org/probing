use pyo3::prelude::*;
use pyo3::types::PyModule;

use probing_core::config;

use crate::features::convert::{ele_to_python, python_to_ele};

/// Helper function to run async config operations from sync Python bindings
fn block_on_async<F, T>(f: F) -> T
where
    F: std::future::Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    match tokio::runtime::Handle::try_current() {
        Ok(_handle) => {
            // We're inside a runtime, spawn a new thread to avoid nested runtime error
            std::thread::spawn(move || {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
                    .block_on(f)
            })
            .join()
            .unwrap()
        }
        Err(_) => {
            // Not in a runtime, create a new one
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(f)
        }
    }
}

/// Get a configuration value.
///
/// Returns None if the key doesn't exist, otherwise returns the value
/// converted to the appropriate Python type.
#[pyfunction]
fn get(py: Python, key: String) -> PyResult<Option<PyObject>> {
    let key_clone = key.clone();
    let ele = block_on_async(async move { config::get(&key_clone).await });
    match ele {
        Some(val) => Ok(Some(ele_to_python(py, &val)?)),
        None => Ok(None),
    }
}

/// Set a configuration value.
///
/// Supports str, int, float, bool, and None values.
#[pyfunction]
fn set(_py: Python, key: String, value: &Bound<'_, PyAny>) -> PyResult<()> {
    let ele = python_to_ele(value)?;
    let key_clone = key.clone();
    block_on_async(async move { config::set(&key_clone, ele).await });
    Ok(())
}

/// Get a configuration value as string.
///
/// Returns None if the key doesn't exist, otherwise returns the value
/// converted to string.
#[pyfunction]
fn get_str(_py: Python, key: String) -> PyResult<Option<String>> {
    let key_clone = key.clone();
    Ok(block_on_async(
        async move { config::get_str(&key_clone).await },
    ))
}

/// Check if a configuration key exists.
#[pyfunction]
fn contains_key(_py: Python, key: String) -> bool {
    let key_clone = key.clone();
    block_on_async(async move { config::contains_key(&key_clone).await })
}

/// Remove a configuration key and return its value.
#[pyfunction]
fn remove(py: Python, key: String) -> PyResult<Option<PyObject>> {
    let key_clone = key.clone();
    let ele = block_on_async(async move { config::remove(&key_clone).await });
    match ele {
        Some(val) => Ok(Some(ele_to_python(py, &val)?)),
        None => Ok(None),
    }
}

/// Get all configuration keys.
#[pyfunction]
fn keys(_py: Python) -> Vec<String> {
    block_on_async(config::keys())
}

/// Clear all configuration.
#[pyfunction]
fn clear(_py: Python) {
    block_on_async(config::clear());
}

/// Get the number of configuration entries.
#[pyfunction]
fn len(_py: Python) -> usize {
    block_on_async(config::len())
}

/// Check if the configuration store is empty.
#[pyfunction]
fn is_empty(_py: Python) -> bool {
    block_on_async(config::is_empty())
}

/// Register the config module to the probing Python module
pub fn register_config_module(parent_module: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = parent_module.py();
    let config_module = PyModule::new(py, "config")?;

    config_module.add_function(wrap_pyfunction!(get, py)?)?;
    config_module.add_function(wrap_pyfunction!(set, py)?)?;
    config_module.add_function(wrap_pyfunction!(get_str, py)?)?;
    config_module.add_function(wrap_pyfunction!(contains_key, py)?)?;
    config_module.add_function(wrap_pyfunction!(remove, py)?)?;
    config_module.add_function(wrap_pyfunction!(keys, py)?)?;
    config_module.add_function(wrap_pyfunction!(clear, py)?)?;
    config_module.add_function(wrap_pyfunction!(len, py)?)?;
    config_module.add_function(wrap_pyfunction!(is_empty, py)?)?;

    parent_module.setattr("config", config_module)?;

    Ok(())
}
