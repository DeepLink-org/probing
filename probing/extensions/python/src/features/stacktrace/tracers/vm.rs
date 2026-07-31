use core::ffi::c_int;

use pyo3::prelude::*;

use probing_proto::prelude::CallFrame;

use crate::features::stacktrace::spy::call::RawCallLocation;
use crate::features::stacktrace::spy::{get_current_frame, get_prev_frame};

use crate::features::stacktrace::spy::ffi;
use crate::features::stacktrace::spy::python_bindings;
use crate::features::stacktrace::spy::with_spy_state;
use crate::features::stacktrace::spy::PYVERSION;

#[allow(static_mut_refs)]
pub fn initialize_globals() -> bool {
    Python::attach(|py| {
        let ver = py.version_info();
        unsafe {
            if PYVERSION.major == 0 {
                PYVERSION = python_bindings::version::Version {
                    major: ver.major as u64,
                    minor: ver.minor as u64,
                    patch: ver.patch as u64,
                    release_flags: ver.suffix.unwrap_or_default().to_string(),
                    build_metadata: Default::default(),
                };
                with_spy_state(|state| {
                    if (*state).stacks.capacity() == 0 {
                        (*state).stacks.reserve(1024);
                    }
                });
                true
            } else {
                false
            }
        }
    })
}

#[allow(static_mut_refs)]
#[inline(always)]
unsafe extern "C" fn rust_eval_frame(
    ts: *mut pyo3::ffi::PyThreadState,
    frame: *mut pyo3::ffi::PyFrameObject,
    extra: c_int,
) -> *mut pyo3::ffi::PyObject {
    with_spy_state(|state| {
        let pid = std::process::id();
        if (*state).published.is_null() || (*state).published_pid != pid {
            (*state).published =
                crate::features::stacktrace::capture::register_python_thread_slot()
                    .map_or(std::ptr::null(), std::ptr::from_ref);
            (*state).published_pid = pid;
        }
        let published = (*state).published.as_ref();

        // Resolve this frame's callee symbol *now*, while the code object is alive
        // under the GIL, and cache it by pointer. The SIGPROF consumer later looks
        // the label up by integer key instead of dereferencing a possibly-freed
        // `PyCodeObject` off the signal path.
        let loc = RawCallLocation::from(frame as usize, Some(ts as usize));
        crate::features::stacktrace::capture::intern_py_frame(&loc);

        (*state).stacks.push(loc);
        if let Some(published) = published {
            published.enter((*state).stacks.len(), loc.callee());
        }

        // macOS default: sample here (no ITIMER_PROF). After PYSTACKS push so
        // the current frame is visible. No-op when async SIGPROF is armed.
        crate::features::stacktrace::tracers::pprof::maybe_cooperative_sample();

        let ret = ((*state).frame_eval)(ts, frame, extra);

        if let Some(published) = published {
            published.leave((*state).stacks.len().saturating_sub(1));
        }
        (*state).stacks.pop();

        ret
    })
}

#[allow(static_mut_refs)]
#[pyfunction]
pub fn enable_tracer() -> PyResult<()> {
    unsafe {
        if PYVERSION.major == 3 && PYVERSION.minor >= 10 {
            let interp = ffi::PyInterpreterState_Get();
            let old_eval_frame = ffi::_PyInterpreterState_GetEvalFrameFunc(interp);
            if old_eval_frame as usize != rust_eval_frame as *const () as usize {
                with_spy_state(|state| {
                    (*state).frame_eval = old_eval_frame;
                });
            }
            ffi::_PyInterpreterState_SetEvalFrameFunc(interp, rust_eval_frame);
        } else {
            return Err(PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!(
                "Python version {}.{} does not support tracer",
                PYVERSION.major, PYVERSION.minor
            )));
        }
    }
    Ok(())
}

/// Whether the eval-frame hook is currently installed. Requires the GIL.
#[allow(static_mut_refs)]
pub fn is_tracer_enabled() -> bool {
    unsafe {
        let interp = ffi::PyInterpreterState_Get();
        let cur = ffi::_PyInterpreterState_GetEvalFrameFunc(interp);
        cur as usize == rust_eval_frame as *const () as usize
    }
}

#[allow(static_mut_refs)]
#[pyfunction]
pub fn disable_tracer() -> PyResult<()> {
    unsafe {
        let interp = ffi::PyInterpreterState_Get();
        let old_eval_frame = ffi::_PyInterpreterState_GetEvalFrameFunc(interp);
        if old_eval_frame as usize == rust_eval_frame as *const () as usize {
            with_spy_state(|state| {
                ffi::_PyInterpreterState_SetEvalFrameFunc(interp, (*state).frame_eval);
                (*state).stacks.clear();
                (*state).stacks.shrink_to_fit();
            });
        }
    }
    crate::features::stacktrace::capture::clear_py_symbols();
    Ok(())
}

#[pyfunction]
pub fn _get_python_stacks(py: Python) -> PyResult<Py<PyAny>> {
    use pyo3::types::{PyDict, PyList};

    let py_list = PyList::empty(py);
    for frame in get_python_stacks_raw() {
        if let CallFrame::PyFrame {
            file, func, lineno, ..
        } = frame
        {
            let dict = PyDict::new(py);
            dict.set_item("file", file)?;
            dict.set_item("func", func)?;
            dict.set_item("lineno", lineno)?;
            py_list.append(dict)?;
        }
    }
    Ok(py_list.into())
}

#[allow(static_mut_refs)]
#[pyfunction]
pub fn _get_python_frames(py: Python) -> PyResult<Py<PyAny>> {
    use pyo3::types::{PyDict, PyList};

    let py_list = PyList::empty(py);

    for frame in get_python_frames_raw(None) {
        if let CallFrame::PyFrame {
            file, func, lineno, ..
        } = frame
        {
            let dict = PyDict::new(py);
            dict.set_item("file", file)?;
            dict.set_item("func", func)?;
            dict.set_item("lineno", lineno)?;
            py_list.append(dict)?;
        }
    }
    Ok(py_list.into())
}

/// Copy TLS `PYSTACKS` callee keys (outer → inner), same order as signal fill.
///
/// Returns `None` when the TLS buffer is unavailable or empty. Interns each
/// key while the GIL holds code objects.
#[allow(static_mut_refs)]
pub fn copy_pystacks_callee_keys() -> Option<Vec<usize>> {
    with_spy_state(|state| unsafe {
        if (*state).stacks.capacity() == 0 {
            return None;
        }
        let keys: Vec<usize> = (*state).stacks.iter().map(|loc| loc.callee()).collect();
        if keys.is_empty() {
            return None;
        }
        for loc in (*state).stacks.iter() {
            crate::features::stacktrace::capture::intern_py_frame(loc);
        }
        Some(keys)
    })
}

#[allow(static_mut_refs)]
pub fn get_python_stacks_raw() -> Vec<CallFrame> {
    with_spy_state(|state| unsafe {
        if (*state).stacks.capacity() == 0 {
            return vec![];
        }
        (*state)
            .stacks
            .iter()
            .rev()
            .filter_map(|location| {
                let location = location.resolve().ok()?;
                Some(CallFrame::PyFrame {
                    file: location.callee.file,
                    func: location.callee.name,
                    lineno: location.callee.line as i64,
                    locals: Default::default(),
                })
            })
            .collect::<Vec<_>>()
    })
}

#[allow(static_mut_refs)]
pub fn get_python_frames_raw(current_frame: Option<usize>) -> Vec<CallFrame> {
    let mut frames = vec![];
    let mut current_frame_addr = match current_frame {
        Some(addr) => Some(addr),
        None => unsafe { get_current_frame(&PYVERSION) },
    };

    if let Some(addr) = current_frame_addr {
        let location = RawCallLocation::from(addr, None).resolve();
        log::debug!("Current frame address: {addr:#x}, location: {location:?}");
        if let Ok(location) = location {
            let filename = location.callee.file;
            let funcname = location.callee.name;
            if filename != "<shim>" || funcname != "<interpreter trampoline>" {
                frames.push(CallFrame::PyFrame {
                    file: filename,
                    func: funcname,
                    lineno: location.callee.line as i64,
                    locals: Default::default(),
                });
            }
        }
    }

    while let Some(addr) = current_frame_addr {
        let location = RawCallLocation::from(addr, None).resolve();
        log::debug!("Current frame address: {addr:#x}, location: {location:?}");
        if let Ok(location) = location {
            if let Some(caller) = location.caller {
                if caller.file != "<shim>" && caller.name != "<interpreter trampoline>" {
                    frames.push(CallFrame::PyFrame {
                        file: caller.file,
                        func: caller.name,
                        lineno: location.lineno as i64,
                        locals: Default::default(),
                    });
                }
            }
            current_frame_addr = unsafe { get_prev_frame(&PYVERSION, addr) };
        }
    }
    frames
}
