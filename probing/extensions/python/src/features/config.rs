use pyo3::prelude::*;
use pyo3::types::PyModule;

use probing_core::config;
use probing_proto::prelude::Ele;

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
        Some(val) => Ok(Some(ele_to_python(py, val)?)),
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

    parent_module.setattr("config", config_module)?;

    Ok(())
}
