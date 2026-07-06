//! Rate-limited warnings when NCCL event pools fill up or exhaust.

use std::sync::atomic::{AtomicI64, Ordering};

use crate::events::now_ns;
use crate::pool::{Indexed, SlotPool};

const WARN_INTERVAL_NS: i64 = 10_000_000_000;

static LAST_USAGE_WARN_NS: AtomicI64 = AtomicI64::new(0);
static LAST_EXHAUST_WARN_NS: AtomicI64 = AtomicI64::new(0);

fn debounce(last: &AtomicI64) -> bool {
    let now = now_ns();
    let prev = last.load(Ordering::Relaxed);
    if now.saturating_sub(prev) < WARN_INTERVAL_NS {
        return false;
    }
    last.store(now, Ordering::Relaxed);
    true
}

pub fn warn_pool_exhausted(pool: &str, usage_pct: u8) {
    if !debounce(&LAST_EXHAUST_WARN_NS) {
        return;
    }
    crate::log::warn(format!(
        "NCCL profiler pool exhausted: {pool} (usage={usage_pct}%) — events dropped; \
         tune PROBING_NCCL_MAX_*_SLOTS or PROBING_NCCL_MIN_MSG_BYTES"
    ));
}

pub fn maybe_warn_pool_usage(pool: &str, usage_pct: u8) {
    if usage_pct < 80 {
        return;
    }
    if debounce(&LAST_USAGE_WARN_NS) {
        crate::log::warn(format!(
            "NCCL profiler pool pressure: {pool} at {usage_pct}% capacity"
        ));
    }
}

pub fn check_pool_usage<T: Indexed>(pool: &str, slot_pool: &SlotPool<T>) {
    maybe_warn_pool_usage(pool, slot_pool.usage_pct());
}
