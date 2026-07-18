//! Python exception crash path: build event → spill → report → grace.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use once_cell::sync::Lazy;
use probing_core::trace::crash_step_snapshot;
use probing_memtable::discover::default_dir;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Mutex;
use uuid::Uuid;

use crate::features::python::tracing::crash_span_snapshot;

use super::grace;
use super::memory_snapshot::{self, MemorySnapshot};
use super::report;
use super::{config, context};

static LAST_COMM: Lazy<Mutex<Option<String>>> = Lazy::new(|| Mutex::new(None));

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CrashEvent {
    pub event_id: String,
    pub timestamp_ns: i64,
    pub kind: String,
    pub rank: i32,
    pub local_rank: i32,
    pub world_size: i32,
    pub host: String,
    pub pid: i32,
    pub exception_type: String,
    pub message: String,
    pub top_frame: String,
    pub traceback: String,
    pub native_backtrace: String,
    pub crash_thread: String,
    pub thread_stacks: String,
    pub fingerprint: String,
    pub global_step: i64,
    pub local_step: i64,
    pub micro_step: i64,
    pub training_phase: String,
    pub active_span: String,
    pub last_comm_op: String,
    pub memory: MemorySnapshot,
}

pub struct CrashInput {
    pub kind: String,
    pub exception_type: String,
    pub message: String,
    pub top_frame: String,
    pub traceback: String,
    pub native_backtrace: String,
    pub crash_thread: String,
    pub thread_stacks: String,
    pub finalize: bool,
}

pub fn note_last_comm(op: &str, _group_size: i32, _bytes: i64, _global_step: i64) {
    if op.is_empty() {
        return;
    }
    if let Ok(mut guard) = LAST_COMM.lock() {
        *guard = Some(op.to_string());
    }
}

pub fn signal_spill_path(pid: i32) -> PathBuf {
    crash_dir().join(pid.to_string()).join("signal-latest.json")
}

pub fn record(input: CrashInput) -> i32 {
    if !config::enabled() {
        return if input.finalize { 1 } else { 0 };
    }

    let ctx = context::snapshot();
    let training = training_context();
    let memory = memory_snapshot::MemorySnapshot::capture();
    let message = truncate(&input.message, 2000);
    let fp = fingerprint(&input.exception_type, &message, &input.top_frame);

    let event = CrashEvent {
        event_id: new_event_id(),
        timestamp_ns: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as i64)
            .unwrap_or(0),
        kind: input.kind,
        rank: ctx.rank,
        local_rank: ctx.local_rank,
        world_size: ctx.world_size,
        host: ctx.host,
        pid: ctx.pid,
        exception_type: input.exception_type,
        message,
        top_frame: input.top_frame,
        traceback: truncate(&input.traceback, 64000),
        native_backtrace: truncate(&input.native_backtrace, 64000),
        crash_thread: input.crash_thread,
        thread_stacks: truncate(&input.thread_stacks, 64000),
        fingerprint: fp,
        global_step: training.global_step,
        local_step: training.local_step,
        micro_step: training.micro_step,
        training_phase: training.training_phase,
        active_span: training.active_span,
        last_comm_op: training.last_comm_op,
        memory,
    };

    let spill_path = if config::spill_enabled() {
        spill_event(&event)
    } else {
        None
    };
    let grace_sec = if input.finalize && grace::should_run_grace() {
        config::grace_sec()
    } else {
        0
    };

    if input.finalize {
        report::print_report(
            &event,
            grace_sec,
            config::force_hold(),
            spill_path.as_deref(),
        );
    } else {
        report::print_summary(&event, spill_path.as_deref());
    }

    if !input.finalize {
        return 1;
    }
    grace::grace_and_maybe_hold(1)
}

struct TrainingContext {
    global_step: i64,
    local_step: i64,
    micro_step: i64,
    training_phase: String,
    active_span: String,
    last_comm_op: String,
}

fn training_context() -> TrainingContext {
    let step = crash_step_snapshot();
    let spans = crash_span_snapshot();
    let comm = LAST_COMM.lock().ok().and_then(|g| g.clone());
    TrainingContext {
        global_step: i64::try_from(step.global_step).unwrap_or(-1),
        local_step: i64::try_from(step.local_step).unwrap_or(-1),
        micro_step: i64::try_from(step.micro_step).unwrap_or(-1),
        training_phase: spans.training_phase.clone(),
        active_span: spans.active_name.clone(),
        last_comm_op: comm.unwrap_or_default(),
    }
}

fn crash_dir() -> PathBuf {
    default_dir().join("crash")
}

fn spill_event(event: &CrashEvent) -> Option<PathBuf> {
    let json = serde_json::to_string(event).ok()?;
    let path = crash_dir()
        .join(event.pid.to_string())
        .join(format!("{}.json", event.event_id));
    write_atomic(&path, json.as_bytes()).ok()?;
    let _ = write_atomic(
        &crash_dir().join(event.pid.to_string()).join("latest.json"),
        json.as_bytes(),
    );
    Some(path)
}

fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(data)?;
        file.sync_all()?;
    }
    fs::rename(tmp, path)
}

fn new_event_id() -> String {
    Uuid::new_v4().simple().to_string()
}

fn truncate(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        text.to_string()
    } else {
        format!("{}...", &text[..limit.saturating_sub(3)])
    }
}

fn fingerprint(exception_type: &str, message: &str, top_frame: &str) -> String {
    let msg = message.trim().replace('\n', " ");
    let msg = if msg.len() <= 120 {
        msg
    } else {
        msg[..120].to_string()
    };
    let payload = format!("{exception_type}|{top_frame}|{msg}");
    let digest = Sha256::digest(payload.as_bytes());
    digest[..8].iter().map(|b| format!("{:02x}", b)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_stable() {
        let a = fingerprint("ValueError", "bad", "train.py:10 in step");
        let b = fingerprint("ValueError", "bad", "train.py:10 in step");
        assert_eq!(a, b);
        assert_eq!(a.len(), 16);
    }

    #[test]
    fn spill_writes_files() {
        let event = CrashEvent {
            event_id: new_event_id(),
            timestamp_ns: 1,
            kind: "python_exception".into(),
            rank: 0,
            local_rank: 0,
            world_size: 1,
            host: "localhost".into(),
            pid: 4242,
            exception_type: "RuntimeError".into(),
            message: "boom".into(),
            top_frame: "t.py:1".into(),
            traceback: String::new(),
            native_backtrace: String::new(),
            crash_thread: String::new(),
            thread_stacks: String::new(),
            fingerprint: "abc".into(),
            global_step: 0,
            local_step: 0,
            micro_step: 0,
            training_phase: String::new(),
            active_span: String::new(),
            last_comm_op: String::new(),
            memory: MemorySnapshot::default(),
        };
        let path = spill_event(&event).expect("spill");
        assert!(path.exists());
    }
}
