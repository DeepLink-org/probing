use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyModule};
use pyo3::IntoPyObjectExt;
use std::cell::RefCell;
use std::sync::{Arc, Mutex};

use probing_core::trace::Span as RawSpan;
use probing_core::trace::{attr, Event as RawEvent, SpanStatus, Timestamp};

use crate::features::convert::{ele_to_python, python_to_ele};

// Thread-local storage for span context
thread_local! {
    static SPAN_STACK: RefCell<Vec<PyObject>> = RefCell::new(Vec::new());
}

/// Python binding for Span
#[pyclass]
#[derive(Clone)]
pub struct Span {
    inner: Arc<Mutex<RawSpan>>,
}

#[pymethods]
impl Span {
    /// Creates a new root span (starts a new trace).
    #[new]
    #[pyo3(signature = (name, *, kind=None, location=None))]
    fn new(name: String, kind: Option<String>, location: Option<String>) -> Self {
        let span = RawSpan::new_root(name, kind.as_deref(), location.as_deref());
        Span {
            inner: Arc::new(Mutex::new(span)),
        }
    }

    /// Creates a new child span from a parent span.
    #[staticmethod]
    #[pyo3(signature = (parent, name, *, kind=None, location=None))]
    fn new_child(
        parent: &Bound<'_, Span>,
        name: String,
        kind: Option<String>,
        location: Option<String>,
    ) -> Self {
        let parent_borrowed = parent.borrow();
        let parent_span = parent_borrowed.inner.lock().unwrap();
        let span = RawSpan::new_child(&*parent_span, name, kind.as_deref(), location.as_deref());
        drop(parent_span);
        Span {
            inner: Arc::new(Mutex::new(span)),
        }
    }

    /// Gets the trace ID.
    #[getter]
    fn trace_id(&self) -> u64 {
        self.inner.lock().unwrap().trace_id
    }

    /// Gets the span ID.
    #[getter]
    fn span_id(&self) -> u64 {
        self.inner.lock().unwrap().span_id
    }

    /// Gets the parent span ID.
    #[getter]
    fn parent_id(&self) -> Option<u64> {
        self.inner.lock().unwrap().parent_id
    }

    /// Gets the originating thread numeric id.
    #[getter]
    fn thread_id(&self) -> u64 {
        self.inner.lock().unwrap().thread_id
    }

    /// Gets the span name.
    #[getter]
    fn name(&self) -> String {
        self.inner.lock().unwrap().name.clone()
    }

    /// Gets the span kind.
    #[getter]
    fn kind(&self) -> Option<String> {
        self.inner.lock().unwrap().kind.clone()
    }

    /// Gets the span status.
    #[getter]
    fn status(&self) -> String {
        match self.inner.lock().unwrap().status() {
            SpanStatus::Active => "Active".to_string(),
            SpanStatus::Completed => "Completed".to_string(),
        }
    }

    /// Checks if the span has been ended.
    #[getter]
    fn is_ended(&self) -> bool {
        self.inner.lock().unwrap().is_ended()
    }

    /// Gets the duration of the span if it has been ended.
    #[getter]
    fn duration(&self) -> Option<f64> {
        self.inner
            .lock()
            .unwrap()
            .duration()
            .map(|d| d.as_secs_f64())
    }

    /// Gets the start timestamp (nanoseconds since epoch).
    #[getter]
    fn start_timestamp(&self) -> u128 {
        self.inner.lock().unwrap().start.0
    }

    /// Gets the end timestamp (nanoseconds since epoch) if the span has been ended.
    #[getter]
    fn end_timestamp(&self) -> Option<u128> {
        self.inner.lock().unwrap().end.map(|t| t.0)
    }

    /// Gets the location from location if available.
    #[getter]
    fn location(&self) -> Option<String> {
        self.inner
            .lock()
            .unwrap()
            .loc
            .as_ref()
            .and_then(|loc| match loc {
                probing_core::trace::Location::UnknownLocation(path) => Some(path.clone()),
                probing_core::trace::Location::KnownLocation(_) => None,
            })
    }

    /// Internal method to set initial attributes during span creation.
    /// This should only be called by the Python wrapper during span creation.
    #[pyo3(name = "_set_initial_attrs")]
    fn set_initial_attrs(&mut self, attrs: &Bound<'_, PyAny>, py: Python) -> PyResult<()> {
        // Convert Python dict to PyDict
        let attrs_dict = attrs.downcast::<PyDict>().map_err(|_| {
            PyErr::new::<pyo3::exceptions::PyTypeError, _>("_set_initial_attrs expects a dict")
        })?;

        let mut inner = self.inner.lock().unwrap();
        for (key, value) in attrs_dict.iter() {
            let key_str = key.extract::<String>()?;
            let ele = python_to_ele(&value)?;
            inner.attrs.push(attr(key_str, ele));
        }
        Ok(())
    }

    /// Adds an event to the span.
    #[pyo3(signature = (name, *, attributes=None))]
    fn add_event(
        &mut self,
        name: String,
        attributes: Option<Vec<PyObject>>,
        py: Python,
    ) -> PyResult<()> {
        let attrs = if let Some(attrs) = attributes {
            let mut converted = Vec::new();
            for attr_obj in attrs {
                // Try to convert attribute object
                if let Ok(dict) = attr_obj.bind(py).downcast::<PyDict>() {
                    for (k, v) in dict.iter() {
                        let key = k.extract::<String>()?;
                        let ele = python_to_ele(&v)?;
                        converted.push(attr(key, ele));
                    }
                } else if let Ok(list) = attr_obj.bind(py).downcast::<PyList>() {
                    if list.len() == 2 {
                        let key = list.get_item(0)?.extract::<String>()?;
                        let value = list.get_item(1)?;
                        let ele = python_to_ele(&value)?;
                        converted.push(attr(key, ele));
                    }
                }
            }
            Some(converted)
        } else {
            None
        };

        self.inner
            .lock()
            .unwrap()
            .add_event(name, attrs)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!("{:?}", e)))?;
        Ok(())
    }

    /// Ends the span.
    fn end(&mut self) {
        self.inner.lock().unwrap().end();
    }

    /// Ends the span with an error message.
    fn end_error(&mut self, error_message: Option<String>) {
        self.inner.lock().unwrap().end_error(error_message);
    }

    /// Gets all attributes as a dictionary.
    fn get_attributes(&self, py: Python) -> PyResult<PyObject> {
        let dict = PyDict::new(py);
        let inner = self.inner.lock().unwrap();
        for attr in &inner.attrs {
            let value = ele_to_python(py, &attr.1)?;
            dict.set_item(&attr.0, value)?;
        }
        Ok(dict.into())
    }

    /// Gets all events as a list.
    fn get_events(&self, py: Python) -> PyResult<PyObject> {
        let list = PyList::empty(py);
        let inner = self.inner.lock().unwrap();
        for event in &inner.events {
            let event_dict = PyDict::new(py);
            event_dict.set_item("name", &event.name)?;
            event_dict.set_item("timestamp", event.timestamp.0 as u64)?;
            let attrs_dict = PyDict::new(py);
            for attr in &event.attributes {
                let value = ele_to_python(py, &attr.1)?;
                attrs_dict.set_item(&attr.0, value)?;
            }
            event_dict.set_item("attributes", attrs_dict)?;
            list.append(event_dict)?;
        }
        Ok(list.into())
    }

    /// Gets an attribute by name (for dynamic attribute access like s.a, s.b).
    fn __getattr__(&self, name: &str, py: Python) -> PyResult<PyObject> {
        // First check if it's a built-in attribute
        match name {
            "trace_id" => return Ok(self.trace_id().into_bound_py_any(py)?.into()),
            "span_id" => return Ok(self.span_id().into_bound_py_any(py)?.into()),
            "parent_id" => {
                if let Some(id) = self.parent_id() {
                    return Ok(id.into_bound_py_any(py)?.into());
                } else {
                    return Ok(py.None());
                }
            }
            "thread_id" => return Ok(self.thread_id().into_bound_py_any(py)?.into()),
            "name" => return Ok(self.name().into_bound_py_any(py)?.into()),
            "kind" => {
                if let Some(k) = self.kind() {
                    return Ok(k.into_bound_py_any(py)?.into());
                } else {
                    return Ok(py.None());
                }
            }
            "status" => return Ok(self.status().into_bound_py_any(py)?.into()),
            "is_ended" => return Ok(self.is_ended().into_bound_py_any(py)?.into()),
            "duration" => {
                if let Some(d) = self.duration() {
                    return Ok(d.into_bound_py_any(py)?.into());
                } else {
                    return Ok(py.None());
                }
            }
            _ => {}
        }

        // Then check if it's in the attributes
        let inner = self.inner.lock().unwrap();
        for attr in &inner.attrs {
            if attr.0 == name {
                let value = ele_to_python(py, &attr.1)?;
                return Ok(value);
            }
        }

        // Not found
        Err(PyErr::new::<pyo3::exceptions::PyAttributeError, _>(
            format!("'Span' object has no attribute '{}'", name),
        ))
    }

    /// Context manager entry (for `with` statement support).
    fn __enter__(slf: PyRef<Self>) -> PyResult<PyRef<Self>> {
        // Push this span to the thread-local stack
        let py = slf.py();
        let span_obj: PyObject = Py::new(py, slf.clone())?.into();
        SPAN_STACK.with(|stack| {
            stack.borrow_mut().push(span_obj);
        });
        Ok(slf)
    }

    /// Context manager exit (for `with` statement support).
    fn __exit__(
        slf: PyRef<Self>,
        _exc_type: Option<&Bound<'_, PyAny>>,
        _exc_val: Option<&Bound<'_, PyAny>>,
        _exc_tb: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<bool> {
        // End the span automatically
        slf.inner.lock().unwrap().end();

        // Pop this span from the stack
        SPAN_STACK.with(|stack| {
            let mut stack = stack.borrow_mut();
            stack.pop();
        });

        Ok(false) // Don't suppress exceptions
    }

    /// Returns a string representation of the span.
    fn __repr__(&self) -> String {
        let inner = self.inner.lock().unwrap();
        format!(
            "Span(name={}, trace_id={}, span_id={}, status={})",
            inner.name,
            inner.trace_id,
            inner.span_id,
            match inner.status() {
                SpanStatus::Active => "Active",
                SpanStatus::Completed => "Completed",
            }
        )
    }
}

// /// Gets the current active span.
// #[pyfunction]
// fn current_span(py: Python) -> PyResult<Option<PyObject>> {
//     SPAN_STACK.with(|stack| {
//         let stack = stack.borrow();
//         Ok(stack.last().cloned())
//     })
// }

/// Gets the current active span.
#[pyfunction]
fn current_span(py: Python) -> PyResult<Option<PyObject>> {
    SPAN_STACK.with(|stack| {
        let stack = stack.borrow();
        Ok(stack.last().map(|obj| obj.clone_ref(py)))
    })
}

/// Internal function to create a span - called by Python wrapper.
/// This is a low-level function that directly creates a span.
#[pyfunction]
#[pyo3(signature = (name, *, kind=None, location=None))]
fn _span_raw(
    py: Python,
    name: String,
    kind: Option<String>,
    location: Option<String>,
) -> PyResult<Span> {
    // Check if there's a current active span
    let parent = SPAN_STACK.with(|stack| {
        let stack = stack.borrow();
        stack.last().map(|obj| obj.clone_ref(py))
    });

    let span = if let Some(parent) = parent {
        // Create a child span
        let parent_obj = parent.bind(py);
        let parent_span = parent_obj.downcast::<Span>()?;
        Span::new_child(parent_span, name, kind, location)
    } else {
        // Create a root span
        Span::new(name, kind, location)
    };

    Ok(span)
}


/// Python binding for Event
#[pyclass]
pub struct Event {
    inner: RawEvent,
}

#[pymethods]
impl Event {
    /// Creates a new event.
    #[new]
    #[pyo3(signature = (name, *, attributes=None))]
    fn new(name: String, attributes: Option<Vec<PyObject>>, py: Python) -> PyResult<Self> {
        let attrs = if let Some(attrs) = attributes {
            let mut converted = Vec::new();
            for attr_obj in attrs {
                if let Ok(dict) = attr_obj.bind(py).downcast::<PyDict>() {
                    for (k, v) in dict.iter() {
                        let key = k.extract::<String>()?;
                        let ele = python_to_ele(&v)?;
                        converted.push(attr(key, ele));
                    }
                } else if let Ok(list) = attr_obj.bind(py).downcast::<PyList>() {
                    if list.len() == 2 {
                        let key = list.get_item(0)?.extract::<String>()?;
                        let value = list.get_item(1)?;
                        let ele = python_to_ele(&value)?;
                        converted.push(attr(key, ele));
                    }
                }
            }
            converted
        } else {
            Vec::new()
        };

        let event = RawEvent {
            name,
            location: None,
            timestamp: Timestamp::now(),
            attributes: attrs,
        };

        Ok(Event { inner: event })
    }

    /// Gets the event name.
    #[getter]
    fn name(&self) -> String {
        self.inner.name.clone()
    }

    /// Gets the event timestamp.
    #[getter]
    fn timestamp(&self) -> u64 {
        self.inner.timestamp.0 as u64
    }

    /// Gets all attributes as a dictionary.
    fn get_attributes(&self, py: Python) -> PyResult<PyObject> {
        let dict = PyDict::new(py);
        for attr in &self.inner.attributes {
            let value = ele_to_python(py, &attr.1)?;
            dict.set_item(&attr.0, value)?;
        }
        Ok(dict.into())
    }

    /// Returns a string representation of the event.
    fn __repr__(&self) -> String {
        format!(
            "Event(name={}, timestamp={})",
            self.inner.name, self.inner.timestamp.0
        )
    }
}

/// Registers the tracing module with Python.
pub fn register_tracing_module(_py: Python, module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<Span>()?;
    module.add_class::<Event>()?;
    module.add_function(wrap_pyfunction!(_span_raw, module)?)?;
    module.add_function(wrap_pyfunction!(current_span, module)?)?;

    // Note: The Python wrapper code in python/probing/tracing.py will import from probing._tracing
    // This module (_tracing) only exposes the raw Rust functions:
    // - Span: The Span class
    // - Event: The Event class
    // - _span_raw: Internal function to create spans
    // - current_span: Function to get current active span
    // The Python wrapper (probing.tracing) will be loaded separately when imported

    Ok(())
}
