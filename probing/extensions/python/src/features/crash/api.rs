//! Python-facing crash API (``probing._core``).

use pyo3::prelude::*;

use super::config;
use super::grace;
use super::handler::{self, CrashInput};

#[pyfunction]
pub fn crash_enabled() -> bool {
    config::enabled()
}

#[pyfunction]
#[allow(clippy::too_many_arguments)] // PyO3 signature mirrors Python excepthook payload
#[pyo3(signature = (kind, exception_type, message, traceback, top_frame, native_backtrace="", finalize=true, thread_stacks="", crash_thread=""))]
pub fn record_crash(
    kind: &str,
    exception_type: &str,
    message: &str,
    traceback: &str,
    top_frame: &str,
    native_backtrace: &str,
    finalize: bool,
    thread_stacks: &str,
    crash_thread: &str,
) -> PyResult<i32> {
    Ok(handler::record(CrashInput {
        kind: kind.to_string(),
        exception_type: exception_type.to_string(),
        message: message.to_string(),
        top_frame: top_frame.to_string(),
        traceback: traceback.to_string(),
        native_backtrace: native_backtrace.to_string(),
        crash_thread: crash_thread.to_string(),
        thread_stacks: thread_stacks.to_string(),
        finalize,
    }))
}

#[pyfunction]
#[pyo3(signature = (op, group_size=0, bytes=0, global_step=-1))]
pub fn note_last_comm(op: &str, group_size: i32, bytes: i64, global_step: i64) {
    handler::note_last_comm(op, group_size, bytes, global_step);
}

#[pyfunction]
pub fn request_crash_hold() {
    grace::request_hold();
}

#[pyfunction]
#[pyo3(signature = (exit_after=true))]
pub fn request_crash_release(exit_after: bool) {
    grace::request_release(exit_after);
}
