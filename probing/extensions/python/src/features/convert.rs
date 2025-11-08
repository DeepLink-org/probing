//! Unified Python-Ele conversion module
//!
//! This module provides centralized conversion functions between Python objects
//! and Ele types, replacing scattered conversion logic throughout the codebase.

use pyo3::prelude::*;
use pyo3::types::{PyBool, PyFloat, PyInt, PyString};
use probing_proto::prelude::Ele;

/// Convert Ele to Python object
///
/// This is the unified implementation that should be used throughout
/// the codebase instead of scattered conversion functions.
pub fn ele_to_python(py: Python, ele: &Ele) -> PyResult<PyObject> {
    let obj: PyObject = match ele {
        Ele::Nil => py.None(),
        Ele::BOOL(b) => PyBool::new(py, *b).to_owned().unbind().into(),
        Ele::I32(i) => PyInt::new(py, *i as i64).to_owned().unbind().into(),
        Ele::I64(i) => PyInt::new(py, *i).to_owned().unbind().into(),
        Ele::F32(f) => PyFloat::new(py, *f as f64).to_owned().unbind().into(),
        Ele::F64(f) => PyFloat::new(py, *f).to_owned().unbind().into(),
        Ele::Text(s) => PyString::new(py, s).to_owned().unbind().into(),
        Ele::Url(s) => PyString::new(py, s).to_owned().unbind().into(),
        Ele::DataTime(t) => {
            // Convert microsecond timestamp to string representation
            use std::time::{Duration, UNIX_EPOCH};
            let datetime = UNIX_EPOCH + Duration::from_micros(*t);
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
///
/// This is the unified implementation that should be used throughout
/// the codebase instead of scattered conversion functions.
pub fn python_to_ele(value: &Bound<'_, PyAny>) -> PyResult<Ele> {
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

