//! Engine initialization lifecycle — readiness for load balancers / orchestrators.

use std::sync::atomic::{AtomicU8, Ordering};

use std::sync::Mutex;

use once_cell::sync::Lazy;

const UNINITIALIZED: u8 = 0;
const READY: u8 = 1;
const FAILED: u8 = 2;
const INITIALIZING: u8 = 3;

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

pub(crate) enum EngineInitClaim {
    Claimed,
    InProgress,
    Ready,
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

/// Claim engine initialization for the current caller.
///
/// A failed initialization may be retried, while an in-flight or completed initialization is
/// left alone. `INITIALIZING` intentionally maps to the existing public `Uninitialized` state.
pub(crate) fn begin_engine_initialization() -> EngineInitClaim {
    let mut state = STATE.load(Ordering::Acquire);
    loop {
        match state {
            READY => return EngineInitClaim::Ready,
            INITIALIZING => return EngineInitClaim::InProgress,
            UNINITIALIZED | FAILED => {
                match STATE.compare_exchange_weak(
                    state,
                    INITIALIZING,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                ) {
                    Ok(_) => return EngineInitClaim::Claimed,
                    Err(observed) => state = observed,
                }
            }
            _ => return EngineInitClaim::InProgress,
        }
    }
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
