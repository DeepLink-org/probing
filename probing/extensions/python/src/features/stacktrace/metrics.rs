//! Process-wide counters for stack capture / aggregation.
//!
//! Two scopes (do not conflate when reading flamegraph JSON):
//!
//! | Group | Counters | When they move |
//! |-------|----------|----------------|
//! | **sampler** | `dropped_*`, `fingerprint_*`, `fold_calls` | SIGPROF consumer; `fold_calls` on **export**, not per sample |
//! | **view** | `parse_calls`, `parse_cache_hits` | demangle/merge path (export + HTTP/dynamic); cache is `(tid,seq)` reuse |

use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::json;

static DROPPED_RING: AtomicU64 = AtomicU64::new(0);
static DROPPED_NOT_MAIN: AtomicU64 = AtomicU64::new(0);
static DROPPED_TORN: AtomicU64 = AtomicU64::new(0);
static DROPPED_CAPACITY: AtomicU64 = AtomicU64::new(0);
static FINGERPRINT_HITS: AtomicU64 = AtomicU64::new(0);
static FINGERPRINT_MISSES: AtomicU64 = AtomicU64::new(0);
static PARSE_CALLS: AtomicU64 = AtomicU64::new(0);
static PARSE_CACHE_HITS: AtomicU64 = AtomicU64::new(0);
static FOLD_CALLS: AtomicU64 = AtomicU64::new(0);

#[inline]
pub fn inc_dropped_ring() {
    DROPPED_RING.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub fn inc_dropped_not_main() {
    DROPPED_NOT_MAIN.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub fn inc_dropped_torn() {
    DROPPED_TORN.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub fn inc_dropped_capacity() {
    DROPPED_CAPACITY.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub fn inc_fingerprint_hit() {
    FINGERPRINT_HITS.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub fn inc_fingerprint_miss() {
    FINGERPRINT_MISSES.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub fn inc_parse_call() {
    PARSE_CALLS.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub fn inc_parse_cache_hit() {
    PARSE_CACHE_HITS.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub fn inc_fold_call() {
    FOLD_CALLS.fetch_add(1, Ordering::Relaxed);
}

pub fn dropped_ring() -> u64 {
    DROPPED_RING.load(Ordering::Relaxed)
}

pub fn fingerprint_hits() -> u64 {
    FINGERPRINT_HITS.load(Ordering::Relaxed)
}

pub fn fingerprint_misses() -> u64 {
    FINGERPRINT_MISSES.load(Ordering::Relaxed)
}

pub fn fold_calls() -> u64 {
    FOLD_CALLS.load(Ordering::Relaxed)
}

pub fn parse_cache_hits() -> u64 {
    PARSE_CACHE_HITS.load(Ordering::Relaxed)
}

pub fn parse_calls() -> u64 {
    PARSE_CALLS.load(Ordering::Relaxed)
}

pub fn dropped_not_main() -> u64 {
    DROPPED_NOT_MAIN.load(Ordering::Relaxed)
}

/// Reset sampler-scope counters (called from pprof `setup` / `reset`).
///
/// Leaves **view** counters (`parse_*`) intact — useful across HTTP polls.
pub fn reset_sampler_counters() {
    DROPPED_RING.store(0, Ordering::Relaxed);
    DROPPED_NOT_MAIN.store(0, Ordering::Relaxed);
    DROPPED_TORN.store(0, Ordering::Relaxed);
    DROPPED_CAPACITY.store(0, Ordering::Relaxed);
    FINGERPRINT_HITS.store(0, Ordering::Relaxed);
    FINGERPRINT_MISSES.store(0, Ordering::Relaxed);
    FOLD_CALLS.store(0, Ordering::Relaxed);
}

/// Counters for flamegraph JSON / debugging, grouped by pipeline scope.
pub fn snapshot_json() -> serde_json::Value {
    json!({
        "sampler": {
            "dropped_ring": DROPPED_RING.load(Ordering::Relaxed),
            "dropped_not_main": DROPPED_NOT_MAIN.load(Ordering::Relaxed),
            "dropped_torn": DROPPED_TORN.load(Ordering::Relaxed),
            "dropped_capacity": DROPPED_CAPACITY.load(Ordering::Relaxed),
            "fingerprint_hits": FINGERPRINT_HITS.load(Ordering::Relaxed),
            "fingerprint_misses": FINGERPRINT_MISSES.load(Ordering::Relaxed),
            "fold_calls": FOLD_CALLS.load(Ordering::Relaxed),
        },
        "view": {
            "parse_calls": PARSE_CALLS.load(Ordering::Relaxed),
            "parse_cache_hits": PARSE_CACHE_HITS.load(Ordering::Relaxed),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_increment() {
        let before = dropped_ring();
        inc_dropped_ring();
        assert!(dropped_ring() > before);
    }

    #[test]
    fn snapshot_json_separates_sampler_and_view() {
        let v = snapshot_json();
        assert!(v["sampler"]["fingerprint_hits"].as_u64().is_some());
        assert!(v["sampler"]["fold_calls"].as_u64().is_some());
        assert!(v["view"]["parse_calls"].as_u64().is_some());
        assert!(v["view"]["parse_cache_hits"].as_u64().is_some());
        // Flat keys removed — do not misread export parse as per-sample work.
        assert!(v.get("fingerprint_hits").is_none());
        assert!(v.get("parse_calls").is_none());
    }
}
