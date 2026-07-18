use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use once_cell::sync::Lazy;
use pyo3::Python;

use probing_proto::prelude::CallFrame;

use probing_core::is_python_main_thread;

use crate::features::stacktrace::capture;
use crate::features::stacktrace::fold::{fold_parsed, FoldOptions, FoldedStacks};
use crate::features::stacktrace::parse::{parse_snapshot, parse_snapshot_cached};
use crate::features::stacktrace::snapshot::{StackFlags, StackSnapshot, StackSource};
use crate::features::stacktrace::tracers::vm;

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
    /// Fill a [`StackSnapshot`] via synchronous backtrace + TLS Python keys.
    pub fn capture_current_snapshot() -> StackSnapshot {
        let mut native = Vec::new();
        backtrace::trace(|frame| {
            native.push(frame.ip() as usize);
            true
        });
        let tid = capture::current_tid();
        let py_snap = capture::copy_registered_py_snapshot(tid);
        let (py_keys, mut flags) = match py_snap {
            Some(s) => {
                let keys: Vec<usize> = s.py[..s.py_len as usize].to_vec();
                (keys, s.flags)
            }
            None => (Vec::new(), StackFlags::PY_ABSENT),
        };
        if !py_keys.is_empty() {
            flags = StackFlags((flags.0) & !StackFlags::PY_ABSENT.0);
        }
        StackSnapshot::from_parts(tid, StackSource::SyncWalk, &native, &py_keys, flags)
    }

    /// Refresh Python keys from live TLS under the GIL, keep native PCs.
    fn refresh_py_keys_under_gil(snap: &StackSnapshot) -> Option<StackSnapshot> {
        let keys = Python::attach(|_py| vm::copy_pystacks_callee_keys())?;
        if keys.is_empty() {
            return None;
        }
        let native: Vec<usize> = snap.native[..snap.native_len as usize].to_vec();
        let mut flags = snap.flags;
        flags = StackFlags(flags.0 & !StackFlags::PY_ABSENT.0 & !StackFlags::PY_TORN.0);
        Some(StackSnapshot::from_parts(
            snap.tid,
            snap.source,
            &native,
            &keys,
            flags,
        ))
    }

    /// Walk the current thread without signals — always through parse.
    pub fn trace_current_thread_merged() -> Vec<CallFrame> {
        let snap = Self::capture_current_snapshot();
        let mut cache = HashMap::new();
        let parsed = parse_snapshot(&snap, &mut cache);
        if !parsed.frames.is_empty() {
            return parsed.frames;
        }
        // Registry miss / empty py: re-copy keys from live PYSTACKS, then parse again.
        if let Some(refreshed) = Self::refresh_py_keys_under_gil(&snap) {
            return parse_snapshot(&refreshed, &mut cache).frames;
        }
        parsed.frames
    }

    fn merged_from_snapshot(snapshot: &StackSnapshot, seq: u64) -> Vec<CallFrame> {
        let mut cache = HashMap::new();
        if seq != 0 {
            parse_snapshot_cached(snapshot, seq, &mut cache).frames
        } else {
            parse_snapshot(snapshot, &mut cache).frames
        }
    }

    /// Read the Python main thread stack from an HTTP / SQL worker.
    ///
    /// Prefer latest SIGPROF mixed samples. **By default we never `SIGUSR2` the
    /// Python main thread** — interrupting libc SIMD routines (e.g. macOS
    /// `_platform_strlen`) has repeatedly resumed as `SIGILL` at a fixed PC when
    /// refreshing Distributed stacks. Opt in with `PROBING_STACK_SIGUSR2_MAIN=1`.
    fn trace_main_thread_off_signal() -> Result<Vec<CallFrame>> {
        let main_tid = capture::python_main_os_tid()
            .ok_or_else(|| anyhow::anyhow!("Python main thread is not registered yet"))?;

        if let Some((snapshot, seq)) = capture::latest_snapshot_with_seq(main_tid) {
            if snapshot.native_len > 0 {
                return Ok(Self::merged_from_snapshot(&snapshot, seq));
            }
        }

        // While SIGPROF is armed, never SIGUSR2 the main thread (Distributed /
        // folded export must drain sampler buckets only). Still allow PYSTACKS.
        if !capture::is_pprof_sampling_active()
            && allow_main_thread_sigusr2()
            && !crate::features::crash::is_crash_held()
        {
            match BACKTRACE_MUTEX.try_lock() {
                Ok(_guard) => {
                    if let Some(snapshot) =
                        capture::capture_thread_snapshot_signal(main_tid, Duration::from_secs(2))
                    {
                        let frames = Self::merged_from_snapshot(&snapshot, 0);
                        if !frames.is_empty() {
                            return Ok(frames);
                        }
                    }
                }
                Err(e) => {
                    log::debug!("main-thread SIGUSR2 skipped (concurrent stack capture): {e}");
                }
            }
        }

        // Safe default: PYSTACKS only (no signal into the training thread).
        if let Some(snapshot) = capture::copy_registered_py_snapshot(main_tid) {
            let frames = Self::merged_from_snapshot(&snapshot, 0);
            if !frames.is_empty() {
                return Ok(frames);
            }
        }

        Err(anyhow::anyhow!(
            "main-thread stack unavailable; enable SIGPROF (probing.pprof.sample_freq) for mixed native+Python stacks"
        ))
    }

    fn is_main_tid(tid: i32) -> bool {
        capture::python_main_os_tid().is_some_and(|main| main == tid as u64)
    }

    fn trace_thread_signal(tid: i32) -> Result<Vec<CallFrame>> {
        if Self::is_main_tid(tid) {
            return Self::trace_main_thread_off_signal();
        }

        if capture::is_pprof_sampling_active() {
            if let Some((snapshot, seq)) = capture::latest_snapshot_with_seq(tid as u64) {
                return Ok(Self::merged_from_snapshot(&snapshot, seq));
            }
        }

        let _guard = BACKTRACE_MUTEX.try_lock().map_err(|e| {
            let busy = BacktraceBusy(e.to_string());
            log::debug!("{busy}; skipping concurrent request");
            anyhow::Error::new(busy)
        })?;

        let snapshot = capture::capture_thread_snapshot_signal(tid as u64, Duration::from_secs(2))
            .ok_or_else(|| {
                anyhow::anyhow!("timed out waiting for stack snapshot on thread {tid}")
            })?;

        Ok(Self::merged_from_snapshot(&snapshot, 0))
    }
}

/// On-demand main-thread stack as a single folded line (`"stack 1"`).
pub fn main_thread_on_demand_folded_lines() -> Vec<String> {
    let frames = match SignalTracer::trace_main_thread_off_signal() {
        Ok(frames) if !frames.is_empty() => frames,
        _ => return Vec::new(),
    };
    let parsed = crate::features::stacktrace::parse::ParsedStacks {
        tid: capture::python_main_os_tid().unwrap_or(0),
        source: StackSource::Vm,
        flags: StackFlags::empty(),
        frames,
    };
    let folded: FoldedStacks = fold_parsed(
        &parsed,
        &FoldOptions {
            thread_prefix: false,
            canonicalize: true,
            count: 1,
        },
    );
    if folded.is_empty() {
        return Vec::new();
    }
    vec![folded.to_folded_line()]
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

/// Opt-in: deliver SIGUSR2 to the Python main OS tid for on-demand mixed stacks.
///
/// Default off — required for Distributed stack refresh stability on Darwin.
fn allow_main_thread_sigusr2() -> bool {
    match std::env::var("PROBING_STACK_SIGUSR2_MAIN") {
        Ok(v) => matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        Err(_) => false,
    }
}
