use pyo3::prelude::*;
use pyo3::types::PyModule;

use probing_core::config::store::ConfigStore;
use probing_proto::prelude::Ele;

/// Get a configuration value.
///
/// Returns None if the key doesn't exist, otherwise returns the value
/// converted to the appropriate Python type.
#[pyfunction]
fn get(py: Python, key: String) -> PyResult<Option<PyObject>> {
    let ele = ConfigStore::get(&key);
    match ele {
        Some(val) => Ok(Some(ele_to_python(py, val)?)),
        None => Ok(None),
    }
}

/// Set a configuration value.
///
/// Supports str, int, float, bool, and None values.
#[pyfunction]
fn set(_py: Python, key: String, value: &Bound<'_, PyAny>) -> PyResult<()> {
    let ele = python_to_ele(value)?;
    ConfigStore::set(&key, ele);
    Ok(())
}

/// Get a configuration value as string.
///
/// Returns None if the key doesn't exist, otherwise returns the value
/// converted to string.
#[pyfunction]
fn get_str(_py: Python, key: String) -> PyResult<Option<String>> {
    Ok(ConfigStore::get_str(&key))
}

/// Check if a configuration key exists.
#[pyfunction]
fn contains_key(_py: Python, key: String) -> bool {
    ConfigStore::contains_key(&key)
}

/// Remove a configuration key and return its value.
#[pyfunction]
fn remove(py: Python, key: String) -> PyResult<Option<PyObject>> {
    let ele = ConfigStore::remove(&key);
    match ele {
        Some(val) => Ok(Some(ele_to_python(py, val)?)),
        None => Ok(None),
    }
}

/// Get all configuration keys.
#[pyfunction]
fn keys(_py: Python) -> Vec<String> {
    ConfigStore::keys()
}

/// Clear all configuration.
#[pyfunction]
fn clear(_py: Python) {
    ConfigStore::clear();
}

/// Get the number of configuration entries.
#[pyfunction]
fn len(_py: Python) -> usize {
    ConfigStore::len()
}

/// Check if the configuration store is empty.
#[pyfunction]
fn is_empty(_py: Python) -> bool {
    ConfigStore::is_empty()
}

/// Get configuration entries with a prefix.
#[pyfunction]
fn get_with_prefix(py: Python, prefix: String) -> PyResult<PyObject> {
    let configs = ConfigStore::get_with_prefix(&prefix);
    let dict = pyo3::types::PyDict::new(py);
    for (k, v) in configs {
        dict.set_item(k, ele_to_python(py, v)?)?;
    }
    Ok(dict.into())
}

/// Remove configuration entries with a prefix.
#[pyfunction]
fn remove_with_prefix(_py: Python, prefix: String) -> usize {
    ConfigStore::remove_with_prefix(&prefix)
}

/// Convert Ele to Python object
fn ele_to_python(py: Python, ele: Ele) -> PyResult<PyObject> {
    use pyo3::types::{PyBool, PyFloat, PyInt, PyString};
    let obj: PyObject = match ele {
        Ele::Nil => py.None(),
        Ele::BOOL(b) => PyBool::new(py, b).to_owned().unbind().into(),
        Ele::I32(i) => PyInt::new(py, i as i64).to_owned().unbind().into(),
        Ele::I64(i) => PyInt::new(py, i).to_owned().unbind().into(),
        Ele::F32(f) => PyFloat::new(py, f as f64).to_owned().unbind().into(),
        Ele::F64(f) => PyFloat::new(py, f).to_owned().unbind().into(),
        Ele::Text(s) => PyString::new(py, &s).to_owned().unbind().into(),
        Ele::Url(s) => PyString::new(py, &s).to_owned().unbind().into(),
        Ele::DataTime(t) => {
            // Convert microsecond timestamp to string representation
            use std::time::{Duration, UNIX_EPOCH};
            let datetime = UNIX_EPOCH + Duration::from_micros(t);
            // Convert to RFC3339 string format (simplified, using chrono-like format)
            // Since we can't use chrono here, we'll use a simple timestamp string
            let s = datetime
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs()
                .to_string();
            PyString::new(py, &s).to_owned().unbind().into()
        }
    };
    Ok(obj)
}

/// Convert Python object to Ele
fn python_to_ele(value: &Bound<'_, PyAny>) -> PyResult<Ele> {
    // Handle None
    if value.is_none() {
        return Ok(Ele::Nil);
    }

    // Try bool
    if let Ok(b) = value.extract::<bool>() {
        return Ok(Ele::BOOL(b));
    }

    // Try int (i64)
    if let Ok(i) = value.extract::<i64>() {
        // Store as I64 for large integers, I32 for smaller ones
        if i >= i32::MIN as i64 && i <= i32::MAX as i64 {
            return Ok(Ele::I32(i as i32));
        }
        return Ok(Ele::I64(i));
    }

    // Try float (f64)
    if let Ok(f) = value.extract::<f64>() {
        // Store as F64 for precision
        return Ok(Ele::F64(f));
    }

    // Try str
    if let Ok(s) = value.extract::<String>() {
        return Ok(Ele::Text(s));
    }

    // Fallback: convert to string
    let s = value.str()?.to_string();
    Ok(Ele::Text(s))
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
    config_module.add_function(wrap_pyfunction!(get_with_prefix, py)?)?;
    config_module.add_function(wrap_pyfunction!(remove_with_prefix, py)?)?;

    parent_module.setattr("config", config_module)?;

    Ok(())
}
