use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyModule};
use pyo3::IntoPyObjectExt;
use std::cell::RefCell;
use std::sync::{Arc, Mutex, MutexGuard};

use probing_core::trace::Span as RawSpan;
use probing_core::trace::{
    advance_micro_step, attr, set_micro_batches, step_snapshot, sync_micro_step, Attribute,
    Event as RawEvent, SpanStatus, StepSnapshot, Timestamp,
};

use crate::features::convert::{ele_to_python, python_to_ele};

fn lock_span(m: &Mutex<RawSpan>) -> MutexGuard<'_, RawSpan> {
    match m.lock() {
        Ok(guard) => guard,
        Err(e) => {
            log::debug!("span mutex poisoned; recovering");
            e.into_inner()
        }
    }
}

fn parse_py_attributes(py: Python, attributes: Vec<Py<PyAny>>) -> PyResult<Vec<Attribute>> {
    let mut converted = Vec::new();
    for attr_obj in attributes {
        if let Ok(dict) = attr_obj.bind(py).cast::<PyDict>() {
            for (k, v) in dict.iter() {
                let key = k.extract::<String>()?;
                let ele = python_to_ele(&v)?;
                converted.push(attr(key, ele));
            }
        } else if let Ok(list) = attr_obj.bind(py).cast::<PyList>() {
            if list.len() == 2 {
                let key = list.get_item(0)?.extract::<String>()?;
                let value = list.get_item(1)?;
                let ele = python_to_ele(&value)?;
                converted.push(attr(key, ele));
            }
        }
    }
    Ok(converted)
}

fn attrs_to_dict(py: Python, attrs: &[Attribute]) -> PyResult<Py<PyAny>> {
    let dict = PyDict::new(py);
    for attr in attrs {
        let value = ele_to_python(py, &attr.1)?;
        dict.set_item(&attr.0, value)?;
    }
    Ok(dict.into())
}

fn optional_into_py<'py, T>(py: Python<'py>, val: Option<T>) -> PyResult<Py<PyAny>>
where
    T: IntoPyObjectExt<'py>,
{
    match val {
        Some(v) => Ok(v.into_bound_py_any(py)?.into()),
        None => Ok(py.None()),
    }
}

// Thread-local storage for span context
thread_local! {
    static SPAN_STACK: RefCell<Vec<Py<PyAny>>> = const { RefCell::new(Vec::new()) };
    static SPAN_HINT_STACK: RefCell<Vec<SpanHint>> = const { RefCell::new(Vec::new()) };
    static CRASH_SPAN_CAPTURE: RefCell<Option<CrashSpanSnapshot>> = const { RefCell::new(None) };
}

#[derive(Clone, Debug, Default)]
struct SpanHint {
    trace_id: u64,
    span_id: u64,
    name: String,
    phase: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct CrashSpanSnapshot {
    pub active_name: String,
    pub trace_id: u64,
    pub span_id: u64,
    pub training_phase: String,
    pub stack: Vec<String>,
}

pub fn crash_span_snapshot() -> CrashSpanSnapshot {
    if let Some(captured) = CRASH_SPAN_CAPTURE.with(|cap| cap.borrow_mut().take()) {
        return captured;
    }
    snapshot_from_hint_stack()
}

fn snapshot_from_hint_stack() -> CrashSpanSnapshot {
    SPAN_HINT_STACK.with(|stack| {
        let stack = stack.borrow();
        if stack.is_empty() {
            return CrashSpanSnapshot::default();
        }
        let names: Vec<String> = stack.iter().map(|h| h.name.clone()).collect();
        let active = stack.last().cloned().unwrap_or_default();
        let training_phase = stack
            .iter()
            .rev()
            .filter_map(|h| h.phase.as_deref())
            .find(|p| matches!(*p, "forward" | "backward" | "optimizer"))
            .unwrap_or("")
            .to_string();
        CrashSpanSnapshot {
            active_name: active.name,
            trace_id: active.trace_id,
            span_id: active.span_id,
            training_phase,
            stack: names,
        }
    })
}

fn capture_span_snapshot_for_crash() {
    let snap = snapshot_from_hint_stack();
    if !snap.active_name.is_empty() {
        CRASH_SPAN_CAPTURE.with(|cap| *cap.borrow_mut() = Some(snap));
    }
}

fn push_span_hint(span: &Span) {
    let hint = SpanHint {
        trace_id: span.with_inner(|s| s.trace_id),
        span_id: span.with_inner(|s| s.span_id),
        name: span.with_inner(|s| s.name.clone()),
        phase: span.with_inner(|s| s.phase.clone()),
    };
    SPAN_HINT_STACK.with(|stack| stack.borrow_mut().push(hint));
}

fn pop_span_hint(span_id: u64) {
    SPAN_HINT_STACK.with(|stack| {
        let mut stack = stack.borrow_mut();
        if let Some(pos) = stack.iter().rposition(|h| h.span_id == span_id) {
            stack.remove(pos);
        }
    });
}

/// Python binding for Span
#[pyclass(from_py_object)]
#[derive(Clone)]
pub struct Span {
    inner: Arc<Mutex<RawSpan>>,
}

impl Span {
    fn with_inner<R>(&self, f: impl FnOnce(&RawSpan) -> R) -> R {
        f(&lock_span(&self.inner))
    }

    fn with_inner_mut<R>(&mut self, f: impl FnOnce(&mut RawSpan) -> R) -> R {
        f(&mut lock_span(&self.inner))
    }
}

#[pymethods]
impl Span {
    /// Creates a new root span (starts a new trace).
    #[new]
    #[pyo3(signature = (name, *, phase=None, location=None))]
    fn new(name: String, phase: Option<String>, location: Option<String>) -> Self {
        let span = RawSpan::new_root(name, phase.as_deref(), location.as_deref());
        Span {
            inner: Arc::new(Mutex::new(span)),
        }
    }

    /// Creates a new child span from a parent span.
    #[staticmethod]
    #[pyo3(signature = (parent, name, *, phase=None, location=None))]
    fn new_child(
        parent: &Bound<'_, Span>,
        name: String,
        phase: Option<String>,
        location: Option<String>,
    ) -> Self {
        let span = parent.borrow().with_inner(|parent_span| {
            RawSpan::new_child(parent_span, name, phase.as_deref(), location.as_deref())
        });
        Span {
            inner: Arc::new(Mutex::new(span)),
        }
    }

    /// Gets the trace ID.
    #[getter]
    fn trace_id(&self) -> u64 {
        self.with_inner(|s| s.trace_id)
    }

    /// Gets the span ID.
    #[getter]
    fn span_id(&self) -> u64 {
        self.with_inner(|s| s.span_id)
    }

    /// Gets the parent span ID.
    #[getter]
    fn parent_id(&self) -> Option<u64> {
        self.with_inner(|s| s.parent_id)
    }

    /// Gets the originating thread numeric id.
    #[getter]
    fn thread_id(&self) -> u64 {
        self.with_inner(|s| s.thread_id)
    }

    /// Gets the span name.
    #[getter]
    fn name(&self) -> String {
        self.with_inner(|s| s.name.clone())
    }

    /// Gets the span training phase (forward / backward / optimizer).
    #[getter]
    fn phase(&self) -> Option<String> {
        self.with_inner(|s| s.phase.clone())
    }

    /// Gets the span status.
    #[getter]
    fn status(&self) -> String {
        match self.with_inner(|s| s.status()) {
            SpanStatus::Active => "Active".to_string(),
            SpanStatus::Completed => "Completed".to_string(),
        }
    }

    /// Checks if the span has been ended.
    #[getter]
    fn is_ended(&self) -> bool {
        self.with_inner(|s| s.is_ended())
    }

    /// Gets the duration of the span if it has been ended.
    #[getter]
    fn duration(&self) -> Option<f64> {
        self.with_inner(|s| s.duration().map(|d| d.as_secs_f64()))
    }

    /// Gets the start timestamp (nanoseconds since epoch).
    #[getter]
    fn start_timestamp(&self) -> u128 {
        self.with_inner(|s| s.start.0)
    }

    /// Gets the end timestamp (nanoseconds since epoch) if the span has been ended.
    #[getter]
    fn end_timestamp(&self) -> Option<u128> {
        self.with_inner(|s| s.end.map(|t| t.0))
    }

    /// Gets the location from location if available.
    #[getter]
    fn location(&self) -> Option<String> {
        self.with_inner(|s| {
            s.loc.as_ref().and_then(|loc| match loc {
                probing_core::trace::Location::UnknownLocation(path) => Some(path.clone()),
                probing_core::trace::Location::KnownLocation(_) => None,
            })
        })
    }

    /// Internal method to set initial attributes during span creation.
    /// This should only be called by the Python wrapper during span creation.
    #[pyo3(name = "_set_initial_attrs")]
    fn set_initial_attrs(&mut self, attrs: &Bound<'_, PyAny>, _py: Python) -> PyResult<()> {
        let attrs_dict = attrs.cast::<PyDict>().map_err(|_| {
            PyErr::new::<pyo3::exceptions::PyTypeError, _>("_set_initial_attrs expects a dict")
        })?;

        self.with_inner_mut(|inner| -> PyResult<()> {
            for (key, value) in attrs_dict.iter() {
                let key_str = key.extract::<String>()?;
                let ele = python_to_ele(&value)?;
                inner.attrs.push(attr(key_str, ele));
            }
            Ok(())
        })
    }

    /// Adds an event to the span.
    #[pyo3(signature = (name, *, attributes=None))]
    fn add_event(
        &mut self,
        name: String,
        attributes: Option<Vec<Py<PyAny>>>,
        py: Python,
    ) -> PyResult<()> {
        let attrs = attributes
            .map(|attrs| parse_py_attributes(py, attrs))
            .transpose()?;

        self.with_inner_mut(|inner| inner.add_event(name, attrs))
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!("{:?}", e)))?;
        Ok(())
    }

    /// Ends the span.
    fn end(&mut self) {
        self.with_inner_mut(|inner| inner.end());
    }

    /// Ends the span with an error message.
    fn end_error(&mut self, error_message: Option<String>) {
        self.with_inner_mut(|inner| inner.end_error(error_message));
    }

    /// Gets all attributes as a dictionary.
    fn get_attributes(&self, py: Python) -> PyResult<Py<PyAny>> {
        self.with_inner(|inner| attrs_to_dict(py, &inner.attrs))
    }

    /// Gets all events as a list.
    fn get_events(&self, py: Python) -> PyResult<Py<PyAny>> {
        let list = PyList::empty(py);
        let inner = lock_span(&self.inner);
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
    fn __getattr__(&self, name: &str, py: Python) -> PyResult<Py<PyAny>> {
        match name {
            "trace_id" => return Ok(self.trace_id().into_bound_py_any(py)?.into()),
            "span_id" => return Ok(self.span_id().into_bound_py_any(py)?.into()),
            "parent_id" => return optional_into_py(py, self.parent_id()),
            "thread_id" => return Ok(self.thread_id().into_bound_py_any(py)?.into()),
            "name" => return Ok(self.name().into_bound_py_any(py)?.into()),
            "phase" => return optional_into_py(py, self.phase()),
            "status" => return Ok(self.status().into_bound_py_any(py)?.into()),
            "is_ended" => return Ok(self.is_ended().into_bound_py_any(py)?.into()),
            "duration" => return optional_into_py(py, self.duration()),
            _ => {}
        }

        if let Some(value) = self.with_inner(|inner| {
            inner
                .attrs
                .iter()
                .find(|attr| attr.0 == name)
                .map(|attr| attr.1.clone())
        }) {
            return ele_to_python(py, &value);
        }

        Err(PyErr::new::<pyo3::exceptions::PyAttributeError, _>(
            format!("'Span' object has no attribute '{name}'"),
        ))
    }

    /// Context manager entry (for `with` statement support).
    fn __enter__(slf: PyRef<Self>) -> PyResult<PyRef<Self>> {
        let py = slf.py();
        push_span_hint(&slf);
        let span_obj: Py<PyAny> = Py::new(py, slf.clone())?.into();
        SPAN_STACK.with(|stack| {
            stack.borrow_mut().push(span_obj);
        });
        Ok(slf)
    }

    /// Context manager exit (for `with` statement support).
    fn __exit__(
        slf: PyRef<Self>,
        exc_type: Option<&Bound<'_, PyAny>>,
        _exc_val: Option<&Bound<'_, PyAny>>,
        _exc_tb: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<bool> {
        if exc_type.is_some() && !exc_type.unwrap().is_none() {
            capture_span_snapshot_for_crash();
        }
        let self_id = slf.span_id();
        lock_span(&slf.inner).end();
        pop_span_hint(self_id);

        SPAN_STACK.with(|stack| {
            let mut stack = stack.borrow_mut();
            if let Some(pos) = stack.iter().rposition(|obj| {
                Python::attach(|py| {
                    obj.bind(py)
                        .cast::<Span>()
                        .ok()
                        .is_some_and(|span| span.borrow().span_id() == self_id)
                })
            }) {
                stack.remove(pos);
            }
        });

        Ok(false)
    }

    /// Returns a string representation of the span.
    fn __repr__(&self) -> String {
        self.with_inner(|inner| {
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
        })
    }
}

/// Gets the current active span.
#[pyfunction]
fn current_span(py: Python) -> PyResult<Option<Py<PyAny>>> {
    SPAN_STACK.with(|stack| {
        let stack = stack.borrow();
        Ok(stack.last().map(|obj| obj.clone_ref(py)))
    })
}

/// Innermost active (non-ended) span for event attachment.
#[pyfunction]
fn active_span_for_events(py: Python) -> PyResult<Option<Py<PyAny>>> {
    SPAN_STACK.with(|stack| {
        let stack = stack.borrow();
        for obj in stack.iter().rev() {
            let is_active = obj
                .bind(py)
                .cast::<Span>()
                .ok()
                .is_some_and(|span| !span.borrow().is_ended());
            if is_active {
                return Ok(Some(obj.clone_ref(py)));
            }
        }
        Ok(None)
    })
}

/// Return the innermost active span whose phase matches ``phase`` (or None).
#[pyfunction]
fn active_span_by_phase(py: Python, phase: String) -> PyResult<Option<Py<PyAny>>> {
    SPAN_STACK.with(|stack| {
        let stack = stack.borrow();
        for obj in stack.iter().rev() {
            let bound = obj.bind(py);
            if let Ok(span) = bound.cast::<Span>() {
                if span.borrow().phase().as_deref() == Some(phase.as_str()) {
                    return Ok(Some(obj.clone_ref(py)));
                }
            }
        }
        Ok(None)
    })
}

/// Innermost active training phase on the span stack (``forward`` / ``backward`` / ``optimizer``).
#[pyfunction]
fn active_training_phase(py: Python) -> PyResult<Option<String>> {
    SPAN_STACK.with(|stack| {
        let stack = stack.borrow();
        for obj in stack.iter().rev() {
            let bound = obj.bind(py);
            if let Ok(span) = bound.cast::<Span>() {
                let borrowed = span.borrow();
                if borrowed.is_ended() {
                    continue;
                }
                if let Some(phase) = borrowed.phase() {
                    match phase.as_str() {
                        "forward" | "backward" | "optimizer" => return Ok(Some(phase)),
                        _ => {}
                    }
                }
            }
        }
        Ok(None)
    })
}

#[pyclass(from_py_object)]
#[derive(Clone, Copy)]
struct PyStepSnapshot {
    #[pyo3(get)]
    micro_step: u64,
    #[pyo3(get)]
    local_step: u64,
    #[pyo3(get)]
    global_step: u64,
    #[pyo3(get)]
    micro_batches: u64,
    #[pyo3(get)]
    rank: i64,
    #[pyo3(get)]
    world_size: i64,
}

impl From<StepSnapshot> for PyStepSnapshot {
    fn from(s: StepSnapshot) -> Self {
        Self {
            micro_step: s.micro_step,
            local_step: s.local_step,
            global_step: s.global_step,
            micro_batches: s.micro_batches,
            rank: s.rank,
            world_size: s.world_size,
        }
    }
}

#[pyfunction]
fn py_step_snapshot() -> PyStepSnapshot {
    step_snapshot().into()
}

#[pyfunction]
fn py_sync_micro_step(step: u64) -> PyStepSnapshot {
    sync_micro_step(step).into()
}

#[pyfunction]
fn py_advance_micro_step() -> PyStepSnapshot {
    advance_micro_step().into()
}

#[pyfunction]
fn py_set_micro_batches(micro_batches: u64) {
    set_micro_batches(micro_batches);
}

#[pyfunction]
fn py_current_micro_step() -> u64 {
    probing_core::trace::current_micro_step()
}

/// Internal function to create a span - called by Python wrapper.
/// This is a low-level function that directly creates a span.
#[pyfunction]
#[pyo3(signature = (name, *, phase=None, location=None))]
fn _span_raw(
    py: Python,
    name: String,
    phase: Option<String>,
    location: Option<String>,
) -> PyResult<Span> {
    let parent = SPAN_STACK.with(|stack| {
        let stack = stack.borrow();
        stack.last().map(|obj| obj.clone_ref(py))
    });

    let span = if let Some(parent) = parent {
        let parent_obj = parent.bind(py);
        let parent_span = parent_obj.cast::<Span>()?;
        Span::new_child(parent_span, name, phase, location)
    } else {
        Span::new(name, phase, location)
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
    fn new(name: String, attributes: Option<Vec<Py<PyAny>>>, py: Python) -> PyResult<Self> {
        let attrs = attributes
            .map(|attrs| parse_py_attributes(py, attrs))
            .transpose()?
            .unwrap_or_default();

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
    fn get_attributes(&self, py: Python) -> PyResult<Py<PyAny>> {
        attrs_to_dict(py, &self.inner.attributes)
    }

    /// Returns a string representation of the event.
    fn __repr__(&self) -> String {
        format!(
            "Event(name={}, timestamp={})",
            self.inner.name, self.inner.timestamp.0
        )
    }
}

pub fn register_tracing_functions(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<Span>()?;
    module.add_class::<Event>()?;
    module.add_class::<PyStepSnapshot>()?;
    module.add_function(wrap_pyfunction!(_span_raw, module)?)?;
    module.add_function(wrap_pyfunction!(current_span, module)?)?;
    module.add_function(wrap_pyfunction!(active_span_for_events, module)?)?;
    module.add_function(wrap_pyfunction!(active_span_by_phase, module)?)?;
    module.add_function(wrap_pyfunction!(active_training_phase, module)?)?;
    module.add_function(wrap_pyfunction!(py_step_snapshot, module)?)?;
    module.add_function(wrap_pyfunction!(py_sync_micro_step, module)?)?;
    module.add_function(wrap_pyfunction!(py_advance_micro_step, module)?)?;
    module.add_function(wrap_pyfunction!(py_set_micro_batches, module)?)?;
    module.add_function(wrap_pyfunction!(py_current_micro_step, module)?)?;

    Ok(())
}
