use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use once_cell::sync::Lazy;
use pyo3::Python;

use probing_proto::prelude::CallFrame;

use probing_core::is_python_main_thread;

use crate::features::stack_capture::{self, RawStackSnapshot};
use crate::features::stack_merge::merged_frames_to_folded_segments;
use crate::features::stack_merge::{demangle_native_symbol, merge_python_native_stacks};
use crate::features::vm_tracer::get_python_stacks_raw;

#[derive(Debug, thiserror::Error)]
#[error("backtrace capture busy: {0}")]
pub(crate) struct BacktraceBusy(String);

pub(crate) fn is_backtrace_busy(err: &anyhow::Error) -> bool {
    err.downcast_ref::<BacktraceBusy>().is_some()
}

#[async_trait]
pub trait StackTracer: Send + Sync + std::fmt::Debug {
    fn trace(&self, tid: Option<i32>) -> Result<Vec<CallFrame>>;
}

#[derive(Debug)]
pub struct SignalTracer;

impl SignalTracer {
    /// Walk the current thread without signals (safe under any Python runtime layout).
    pub fn trace_current_thread_merged() -> Vec<CallFrame> {
        let mut native_leaf_to_root = Vec::new();
        backtrace::trace(|frame| {
            let ip = frame.ip();
            backtrace::resolve_frame(frame, |symbol| {
                let symbol_address = symbol.addr().unwrap_or(ip);
                let (func_name, lang) = symbol
                    .name()
                    .and_then(|name| name.as_str())
                    .map(demangle_native_symbol)
                    .unwrap_or_else(|| (format!("unknown@{symbol_address:p}"), None));
                let file_name = symbol
                    .filename()
                    .map(|path| path.to_string_lossy().into_owned())
                    .unwrap_or_default();
                native_leaf_to_root.push(CallFrame::CFrame {
                    ip: format!("{ip:p}"),
                    file: file_name,
                    func: func_name,
                    lineno: symbol.lineno().unwrap_or(0) as i64,
                    lang: lang.map(str::to_string),
                });
            });
            true
        });

        let python = Python::attach(|_py| get_python_stacks_raw());
        if python.is_empty() {
            native_leaf_to_root.reverse();
            return native_leaf_to_root;
        }
        if native_leaf_to_root.is_empty() {
            return python;
        }
        merge_python_native_stacks(&python, &native_leaf_to_root)
    }

    fn merged_from_snapshot(snapshot: &RawStackSnapshot) -> Vec<CallFrame> {
        let mut cache = HashMap::new();
        stack_capture::snapshot_to_merged_frames(snapshot, &mut cache)
    }

    /// Read the Python main thread stack without SIGUSR2 (safe from HTTP / SQL workers).
    fn trace_main_thread_off_signal() -> Result<Vec<CallFrame>> {
        let main_tid = stack_capture::python_main_os_tid()
            .ok_or_else(|| anyhow::anyhow!("Python main thread is not registered yet"))?;

        if let Some(snapshot) = stack_capture::latest_snapshot_for_tid(main_tid) {
            return Ok(Self::merged_from_snapshot(&snapshot));
        }

        if let Some(snapshot) = stack_capture::copy_registered_py_snapshot(main_tid) {
            let frames = Self::merged_from_snapshot(&snapshot);
            if !frames.is_empty() {
                return Ok(frames);
            }
        }

        Err(anyhow::anyhow!(
            "main-thread stack unavailable; enable SIGPROF sampling or retry while training is active"
        ))
    }

    fn is_main_tid(tid: i32) -> bool {
        stack_capture::python_main_os_tid().is_some_and(|main| main == tid as u64)
    }

    fn trace_thread_signal(tid: i32) -> Result<Vec<CallFrame>> {
        if Self::is_main_tid(tid) {
            return Self::trace_main_thread_off_signal();
        }

        if stack_capture::is_pprof_sampling_active() {
            if let Some(snapshot) = stack_capture::latest_snapshot_for_tid(tid as u64) {
                return Ok(Self::merged_from_snapshot(&snapshot));
            }
        }

        let _guard = BACKTRACE_MUTEX.try_lock().map_err(|e| {
            let busy = BacktraceBusy(e.to_string());
            log::debug!("{busy}; skipping concurrent request");
            anyhow::Error::new(busy)
        })?;

        let snapshot =
            stack_capture::capture_thread_snapshot_signal(tid as u64, Duration::from_secs(2))
                .ok_or_else(|| {
                    anyhow::anyhow!("timed out waiting for stack snapshot on thread {tid}")
                })?;

        Ok(Self::merged_from_snapshot(&snapshot))
    }
}

/// On-demand main-thread stack as a single folded line (`"stack 1"`).
pub fn main_thread_on_demand_folded_lines() -> Vec<String> {
    let frames = match SignalTracer::trace_main_thread_off_signal() {
        Ok(frames) if !frames.is_empty() => frames,
        _ => return Vec::new(),
    };
    let segments = merged_frames_to_folded_segments(&frames);
    if segments.is_empty() {
        return Vec::new();
    }
    vec![format!("{} 1", segments.join(";"))]
}

#[async_trait]
impl StackTracer for SignalTracer {
    fn trace(&self, tid: Option<i32>) -> Result<Vec<CallFrame>> {
        log::debug!("Collecting backtrace for TID: {tid:?}");

        let explicit = tid.filter(|&t| t > 0);

        if explicit.is_none() {
            if is_python_main_thread() {
                return Ok(Self::trace_current_thread_merged());
            }
            return Self::trace_main_thread_off_signal();
        }

        let target = explicit.unwrap();
        if Self::is_main_tid(target) {
            return Self::trace_main_thread_off_signal();
        }

        match catch_unwind(AssertUnwindSafe(|| Self::trace_thread_signal(target))) {
            Ok(Ok(frames)) => Ok(frames),
            Ok(Err(err)) => {
                if is_backtrace_busy(&err) {
                    log::debug!("Cross-thread stack trace skipped for tid {target}: {err}");
                } else {
                    log::warn!("Cross-thread stack trace failed for tid {target}: {err}");
                }
                Err(err)
            }
            Err(_) => {
                log::warn!("Cross-thread stack trace panicked for tid {target}");
                Err(anyhow::anyhow!(
                    "cross-thread stack trace panicked for tid {target}"
                ))
            }
        }
    }
}

static BACKTRACE_MUTEX: Lazy<tokio::sync::Mutex<()>> = Lazy::new(|| tokio::sync::Mutex::new(()));
