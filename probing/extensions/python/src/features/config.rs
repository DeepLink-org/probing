use pyo3::prelude::*;
use pyo3::types::PyModule;

use probing_core::config;
use probing_core::runtime::block_on;

use crate::features::convert::{ele_to_python, python_to_ele};

/// Get a configuration value.
///
/// Returns None if the key doesn't exist, otherwise returns the value
/// converted to the appropriate Python type.
#[pyfunction(name = "config_get")]
fn get(py: Python, key: String) -> PyResult<Option<Py<PyAny>>> {
    let key_clone = key.clone();
    let ele = py.detach(|| block_on(async move { config::get(&key_clone).await }));
    match ele {
        Some(val) => Ok(Some(ele_to_python(py, &val)?)),
        None => Ok(None),
    }
}

/// Set a configuration value.
///
/// Supports str, int, float, bool, and None values.
#[pyfunction(name = "config_set")]
fn set(py: Python, key: String, value: &Bound<'_, PyAny>) -> PyResult<()> {
    let ele = python_to_ele(value)?;
    let key_clone = key.clone();
    py.detach(|| block_on(async move { config::set(&key_clone, ele).await }));
    Ok(())
}

/// Get a configuration value as string.
///
/// Returns None if the key doesn't exist, otherwise returns the value
/// converted to string.
#[pyfunction(name = "config_get_str")]
fn get_str(py: Python, key: String) -> PyResult<Option<String>> {
    let key_clone = key.clone();
    Ok(py.detach(|| block_on(async move { config::get_str(&key_clone).await })))
}

/// Check if a configuration key exists.
#[pyfunction(name = "config_contains_key")]
fn contains_key(py: Python, key: String) -> bool {
    let key_clone = key.clone();
    py.detach(|| block_on(async move { config::contains_key(&key_clone).await }))
}

/// Remove a configuration key and return its value.
#[pyfunction(name = "config_remove")]
fn remove(py: Python, key: String) -> PyResult<Option<Py<PyAny>>> {
    let key_clone = key.clone();
    let ele = py.detach(|| block_on(async move { config::remove(&key_clone).await }));
    match ele {
        Some(val) => Ok(Some(ele_to_python(py, &val)?)),
        None => Ok(None),
    }
}

/// Get all configuration keys.
#[pyfunction(name = "config_keys")]
fn keys(py: Python) -> Vec<String> {
    py.detach(|| block_on(config::keys()))
}

/// Clear all configuration.
#[pyfunction(name = "config_clear")]
fn clear(py: Python) {
    py.detach(|| block_on(config::clear()));
}

/// Get the number of configuration entries.
#[pyfunction(name = "config_len")]
fn len(py: Python) -> usize {
    py.detach(|| block_on(config::len()))
}

/// Check if the configuration store is empty.
#[pyfunction(name = "config_is_empty")]
fn is_empty(py: Python) -> bool {
    py.detach(|| block_on(config::is_empty()))
}

/// Register the config functions directly to the probing Python module.
pub fn register_config_functions(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(get, module)?)?;
    module.add_function(wrap_pyfunction!(set, module)?)?;
    module.add_function(wrap_pyfunction!(get_str, module)?)?;
    module.add_function(wrap_pyfunction!(contains_key, module)?)?;
    module.add_function(wrap_pyfunction!(remove, module)?)?;
    module.add_function(wrap_pyfunction!(keys, module)?)?;
    module.add_function(wrap_pyfunction!(clear, module)?)?;
    module.add_function(wrap_pyfunction!(len, module)?)?;
    module.add_function(wrap_pyfunction!(is_empty, module)?)?;

    Ok(())
}
