//! Engine initialization lifecycle — readiness for load balancers / orchestrators.

use std::sync::atomic::{AtomicU8, Ordering};

use std::sync::Mutex;

use once_cell::sync::Lazy;

const UNINITIALIZED: u8 = 0;
const READY: u8 = 1;
const FAILED: u8 = 2;

static STATE: AtomicU8 = AtomicU8::new(UNINITIALIZED);
static FAIL_REASON: Lazy<Mutex<Option<String>>> = Lazy::new(|| Mutex::new(None));

fn lock_fail_reason() -> std::sync::MutexGuard<'static, Option<String>> {
    FAIL_REASON.lock().unwrap_or_else(|e| e.into_inner())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineInitState {
    Uninitialized,
    Ready,
    Failed(String),
}

pub fn mark_engine_ready() {
    *lock_fail_reason() = None;
    STATE.store(READY, Ordering::Release);
}

pub fn mark_engine_failed(reason: impl Into<String>) {
    let reason = reason.into();
    *lock_fail_reason() = Some(reason.clone());
    STATE.store(FAILED, Ordering::Release);
}

pub fn engine_init_state() -> EngineInitState {
    match STATE.load(Ordering::Acquire) {
        READY => EngineInitState::Ready,
        FAILED => EngineInitState::Failed(
            lock_fail_reason()
                .clone()
                .unwrap_or_else(|| "engine initialization failed".into()),
        ),
        _ => EngineInitState::Uninitialized,
    }
}

pub fn engine_is_ready() -> bool {
    STATE.load(Ordering::Acquire) == READY
}

pub fn engine_not_ready_message() -> Option<String> {
    match engine_init_state() {
        EngineInitState::Ready => None,
        EngineInitState::Uninitialized => Some("engine not initialized yet".into()),
        EngineInitState::Failed(reason) => Some(format!("engine initialization failed: {reason}")),
    }
}

pub fn engine_fail_fast_enabled() -> bool {
    matches!(
        std::env::var("PROBING_ENGINE_FAIL_FAST")
            .ok()
            .as_deref()
            .map(str::trim),
        Some("1") | Some("true") | Some("TRUE") | Some("on") | Some("ON")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ready_roundtrip() {
        mark_engine_ready();
        assert!(engine_is_ready());
        assert_eq!(engine_init_state(), EngineInitState::Ready);
        assert!(engine_not_ready_message().is_none());
    }

    #[test]
    fn failed_surfaces_reason() {
        mark_engine_failed("memtable missing");
        assert!(!engine_is_ready());
        let msg = engine_not_ready_message().unwrap();
        assert!(msg.contains("memtable missing"));
    }
}
