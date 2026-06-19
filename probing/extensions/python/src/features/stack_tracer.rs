use std::collections::HashSet;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::mpsc;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use lazy_static::lazy_static;
use nix::libc;
use once_cell::sync::Lazy;
use pyo3::Python;

use probing_proto::prelude::CallFrame;

use probing_core::is_python_main_thread;

use crate::features::vm_tracer::{get_python_frames_raw, get_python_stacks_raw};

fn demangle_native_symbol(raw_name: &str) -> (String, Option<&'static str>) {
    if let Ok(d) = rustc_demangle::try_demangle(raw_name) {
        return (d.to_string(), Some("rust"));
    }
    // macOS C ABI adds a leading `_` to Rust v0 mangling (`_R...` -> `__R...`).
    if raw_name.starts_with("__R") {
        if let Ok(d) = rustc_demangle::try_demangle(&raw_name[1..]) {
            return (d.to_string(), Some("rust"));
        }
    }
    if let Some(demangled) = cpp_demangle::Symbol::new(raw_name)
        .ok()
        .and_then(|sym| sym.demangle().ok())
    {
        return (demangled, Some("cpp"));
    }
    (raw_name.to_string(), None)
}

lazy_static! {
    static ref WHITELISTED_PREFIXES: HashSet<&'static str> = {
        const PREFIXES: &[&str] = &[
            "time",
            "sys",
            "gc",
            "os",
            "unicode",
            "thread",
            "stringio",
            "sre",
            "PyGilState",
            "PyThread",
            "lock",
        ];
        PREFIXES.iter().copied().collect()
    };
}

#[derive(Copy, Clone)]
enum MergeType {
    Ignore,
    MergeNativeFrame,
    MergePythonFrame,
}

fn merge_strategy(frame: &CallFrame) -> MergeType {
    let symbol = match frame {
        CallFrame::CFrame { func, .. } => func,
        CallFrame::PyFrame { func, .. } => func,
    };
    let mut tokens = symbol.split(['_', '.']).filter(|s| !s.is_empty());
    match tokens.next() {
        Some("PyEval") => match tokens.next() {
            Some("EvalFrameDefault" | "EvalFrameEx") => MergeType::MergePythonFrame,
            _ => MergeType::Ignore,
        },
        Some(prefix) if WHITELISTED_PREFIXES.contains(prefix) => MergeType::MergeNativeFrame,
        _ => MergeType::MergeNativeFrame,
    }
}

#[async_trait]
pub trait StackTracer: Send + Sync + std::fmt::Debug {
    fn trace(&self, tid: Option<i32>) -> Result<Vec<CallFrame>>;
}

#[derive(Debug)]
pub struct SignalTracer;

impl SignalTracer {
    fn get_native_stacks() -> Option<Vec<CallFrame>> {
        let mut frames = vec![];
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
                frames.push(CallFrame::CFrame {
                    ip: format!("{ip:p}"),
                    file: file_name,
                    func: func_name,
                    lineno: symbol.lineno().unwrap_or(0) as i64,
                    lang: lang.map(str::to_string),
                });
            });
            true
        });
        Some(frames)
    }

    fn send_frames(frames: Vec<CallFrame>) -> Result<()> {
        match NATIVE_CALLSTACK_SENDER_SLOT.try_lock() {
            Ok(guard) => {
                if let Some(sender) = guard.as_ref() {
                    sender.send(frames)?;
                    Ok(())
                } else {
                    Err(anyhow::anyhow!("No sender available in channel slot"))
                }
            }
            Err(_) => Err(anyhow::anyhow!("Failed to send frames via channel")),
        }
    }

    fn merge_python_native_stacks(
        python_stacks: Vec<CallFrame>,
        native_stacks: Vec<CallFrame>,
    ) -> Vec<CallFrame> {
        let mut merged = vec![];
        let mut python_frame_index = 0;

        for frame in native_stacks {
            match merge_strategy(&frame) {
                MergeType::Ignore => {}
                MergeType::MergeNativeFrame => merged.push(frame),
                MergeType::MergePythonFrame => {
                    if let Some(py_frame) = python_stacks.get(python_frame_index) {
                        merged.push(py_frame.clone());
                    }
                    python_frame_index += 1;
                }
            }
        }
        merged
    }

    /// Walk the current thread without signals (safe under any Python runtime layout).
    pub fn trace_current_thread_merged() -> Result<Vec<CallFrame>> {
        let native = Self::get_native_stacks().unwrap_or_default();
        let python = Python::attach(|_py| {
            let from_tracer = get_python_stacks_raw();
            if !from_tracer.is_empty() {
                from_tracer
            } else {
                get_python_frames_raw(None)
            }
        });
        if python.is_empty() {
            return Ok(native);
        }
        if native.is_empty() {
            return Ok(python);
        }
        Ok(Self::merge_python_native_stacks(python, native))
    }

    fn default_signal_tid() -> i32 {
        nix::unistd::getpid().as_raw()
    }

    fn clear_sender_slot() {
        if let Ok(mut guard) = NATIVE_CALLSTACK_SENDER_SLOT.try_lock() {
            guard.take();
        }
    }

    fn trace_thread_signal(tid: i32) -> Result<Vec<CallFrame>> {
        let pid = nix::unistd::getpid().as_raw();
        let tid_param = Some(tid);

        let _guard = BACKTRACE_MUTEX.try_lock().map_err(|e| {
            log::error!("Failed to acquire BACKTRACE_MUTEX: {e}");
            anyhow::anyhow!("Failed to acquire backtrace lock: {}", e)
        })?;

        let (tx, rx) = mpsc::channel::<Vec<CallFrame>>();
        NATIVE_CALLSTACK_SENDER_SLOT
            .try_lock()
            .map_err(|err| {
                log::error!("Failed to lock CALLSTACK_SENDER_SLOT: {err}");
                anyhow::anyhow!("Failed to lock call stack sender slot")
            })?
            .replace(tx);

        log::debug!("Sending SIGUSR2 signal to process {pid} (thread: {tid})");

        #[cfg(target_os = "linux")]
        {
            let ret = unsafe { libc::syscall(libc::SYS_tgkill, pid, tid, libc::SIGUSR2) };
            if ret != 0 {
                let last_error = std::io::Error::last_os_error();
                Self::clear_sender_slot();
                return Err(anyhow::anyhow!(
                    "Failed to send SIGUSR2 to process {pid} (thread: {tid}): {last_error}"
                ));
            }
        }

        #[cfg(target_os = "macos")]
        {
            let signal_result = if tid_param.is_none() || tid == pid {
                let ret = unsafe { libc::kill(pid, libc::SIGUSR2) };
                if ret != 0 {
                    Err(std::io::Error::last_os_error())
                } else {
                    Ok(())
                }
            } else {
                probing_cc::extensions::send_sigusr2_to_thread_id(tid)
            };
            if let Err(e) = signal_result {
                Self::clear_sender_slot();
                return Err(anyhow::anyhow!(
                    "Failed to send SIGUSR2 to process {pid} (thread: {tid}): {e}"
                ));
            }
        }

        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            let _ = (pid, tid, tid_param);
            Self::clear_sender_slot();
            return Err(anyhow::anyhow!(
                "Stack tracing is not supported on this platform"
            ));
        }

        let native_frames = match rx.recv_timeout(Duration::from_secs(2)) {
            Ok(frames) => frames,
            Err(err) => {
                Self::clear_sender_slot();
                return Err(err.into());
            }
        };
        let python_frames = rx.recv_timeout(Duration::from_secs(2)).unwrap_or_default();
        Self::clear_sender_slot();

        Ok(Self::merge_python_native_stacks(
            python_frames,
            native_frames,
        ))
    }
}

#[async_trait]
impl StackTracer for SignalTracer {
    fn trace(&self, tid: Option<i32>) -> Result<Vec<CallFrame>> {
        log::debug!("Collecting backtrace for TID: {tid:?}");

        // The CPU sampler now uses an async-signal-safe SIGPROF handler that does
        // no heavy work, so it no longer conflicts with SIGUSR2 stack capture.

        if tid.is_none() && is_python_main_thread() {
            return Self::trace_current_thread_merged();
        }

        let target = tid.unwrap_or_else(Self::default_signal_tid);
        match catch_unwind(AssertUnwindSafe(|| Self::trace_thread_signal(target))) {
            Ok(Ok(frames)) => Ok(frames),
            Ok(Err(err)) => {
                log::warn!("Cross-thread stack trace failed for tid {target}: {err}");
                Ok(vec![])
            }
            Err(_) => {
                log::warn!("Cross-thread stack trace panicked for tid {target}");
                Self::clear_sender_slot();
                Ok(vec![])
            }
        }
    }
}

pub fn backtrace_signal_handler() {
    // Runs on the signaled thread: native unwind + thread-local eval-frame tracer stack.
    let native_stacks = SignalTracer::get_native_stacks().unwrap_or_default();
    if SignalTracer::send_frames(native_stacks).is_err() {
        log::error!("Signal handler: failed to send native stacks (receiver may have timed out)");
    }
    let python_stacks = get_python_stacks_raw();
    if SignalTracer::send_frames(python_stacks).is_err() {
        log::error!("Signal handler: failed to send Python stacks from eval tracer");
    }
}

/// Define a static Mutex for the backtrace function
static BACKTRACE_MUTEX: Lazy<tokio::sync::Mutex<()>> = Lazy::new(|| tokio::sync::Mutex::new(()));

pub static NATIVE_CALLSTACK_SENDER_SLOT: Lazy<Mutex<Option<mpsc::Sender<Vec<CallFrame>>>>> =
    Lazy::new(|| Mutex::new(None));
