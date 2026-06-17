use pyo3::prelude::*;
use pyo3::types::PyModule;

use probing_core::config;
use probing_core::runtime::block_on;

use crate::features::convert::{ele_to_python, python_to_ele};
use crate::features::native_bridge::with_detached_native;

/// Get a configuration value.
///
/// Returns None if the key doesn't exist, otherwise returns the value
/// converted to the appropriate Python type.
#[pyfunction(name = "config_get")]
fn get(_py: Python, key: String) -> PyResult<Option<Py<PyAny>>> {
    with_detached_native(move || {
        Python::attach(|py| {
            let ele = block_on(async move { config::get(&key).await });
            match ele {
                Some(val) => Ok(Some(ele_to_python(py, &val)?)),
                None => Ok(None),
            }
        })
    })
}

/// Set a configuration value.
///
/// Supports str, int, float, bool, and None values.
#[pyfunction(name = "config_set")]
fn set(_py: Python, key: String, value: Bound<'_, PyAny>) -> PyResult<()> {
    let value = value.unbind();
    with_detached_native(move || {
        Python::attach(|py| {
            let ele = python_to_ele(value.bind(py))?;
            block_on(async move { config::set(&key, ele).await });
            Ok(())
        })
    })
}

/// Get a configuration value as string.
///
/// Returns None if the key doesn't exist, otherwise returns the value
/// converted to string.
#[pyfunction(name = "config_get_str")]
fn get_str(_py: Python, key: String) -> PyResult<Option<String>> {
    Ok(with_detached_native(move || block_on(async move { config::get_str(&key).await })))
}

/// Check if a configuration key exists.
#[pyfunction(name = "config_contains_key")]
fn contains_key(_py: Python, key: String) -> bool {
    with_detached_native(move || block_on(async move { config::contains_key(&key).await }))
}

/// Remove a configuration key and return its value.
#[pyfunction(name = "config_remove")]
fn remove(_py: Python, key: String) -> PyResult<Option<Py<PyAny>>> {
    with_detached_native(move || {
        Python::attach(|py| {
            let ele = block_on(async move { config::remove(&key).await });
            match ele {
                Some(val) => Ok(Some(ele_to_python(py, &val)?)),
                None => Ok(None),
            }
        })
    })
}

/// Get all configuration keys.
#[pyfunction(name = "config_keys")]
fn keys(_py: Python) -> Vec<String> {
    with_detached_native(|| block_on(config::keys()))
}

/// Clear all configuration.
#[pyfunction(name = "config_clear")]
fn clear(_py: Python) {
    with_detached_native(|| block_on(config::clear()));
}

/// Get the number of configuration entries.
#[pyfunction(name = "config_len")]
fn len(_py: Python) -> usize {
    with_detached_native(|| block_on(config::len()))
}

/// Check if the configuration store is empty.
#[pyfunction(name = "config_is_empty")]
fn is_empty(_py: Python) -> bool {
    with_detached_native(|| block_on(config::is_empty()))
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
