//! [`StackSnapshot`] → [`ParsedStacks`]: demangle + Python resolve + merge.
//!
//! Output frame order is always root → leaf (caller → callee). This path is a
//! warm/cold path — not for SIGPROF handlers.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use once_cell::sync::Lazy;
use probing_proto::prelude::CallFrame;

use crate::features::stacktrace::capture::{resolve_py_call_frame, symbolize_native_addr};
use crate::features::stacktrace::merge::merge_python_native_stacks;
use crate::features::stacktrace::metrics;
use crate::features::stacktrace::snapshot::{StackFlags, StackSnapshot, StackSource};

/// Small FIFO of recent `(tid, seq)` parses for HTTP / dynamic reuse.
const VIEW_CACHE_CAP: usize = 16;

struct ParsedViewCache {
    /// Oldest at front; capacity capped at [`VIEW_CACHE_CAP`].
    entries: VecDeque<(u64, u64, ParsedStacks)>,
}

impl ParsedViewCache {
    fn new() -> Self {
        Self {
            entries: VecDeque::with_capacity(VIEW_CACHE_CAP),
        }
    }

    fn get(&self, tid: u64, seq: u64) -> Option<&ParsedStacks> {
        self.entries
            .iter()
            .find(|(t, s, _)| *t == tid && *s == seq)
            .map(|(_, _, p)| p)
    }

    fn insert(&mut self, tid: u64, seq: u64, parsed: ParsedStacks) {
        if let Some(pos) = self
            .entries
            .iter()
            .position(|(t, s, _)| *t == tid && *s == seq)
        {
            self.entries.remove(pos);
        }
        while self.entries.len() >= VIEW_CACHE_CAP {
            self.entries.pop_front();
        }
        self.entries.push_back((tid, seq, parsed));
    }
}

static PARSED_VIEW_CACHE: Lazy<Mutex<ParsedViewCache>> =
    Lazy::new(|| Mutex::new(ParsedViewCache::new()));

/// Symbolized and merged stack (root → leaf).
#[derive(Clone, Debug)]
pub struct ParsedStacks {
    pub tid: u64,
    pub source: StackSource,
    pub flags: StackFlags,
    pub frames: Vec<CallFrame>,
}

impl ParsedStacks {
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }
}

/// Parse a snapshot: native symbolize, Python intern lookup, merge.
///
/// `cache` is reused across samples on the consumer thread (native demangle).
pub fn parse_snapshot(
    snapshot: &StackSnapshot,
    cache: &mut HashMap<usize, CallFrame>,
) -> ParsedStacks {
    metrics::inc_parse_call();
    parse_snapshot_uncached(snapshot, cache)
}

fn parse_snapshot_uncached(
    snapshot: &StackSnapshot,
    cache: &mut HashMap<usize, CallFrame>,
) -> ParsedStacks {
    if snapshot.flags.contains(StackFlags::PY_TORN) {
        return ParsedStacks {
            tid: snapshot.tid,
            source: snapshot.source,
            flags: snapshot.flags,
            frames: Vec::new(),
        };
    }

    let nlen = snapshot.native_len as usize;
    let plen = snapshot.py_len as usize;

    let native_leaf_to_root: Vec<CallFrame> = (0..nlen)
        .map(|i| {
            let resolve_addr = if i == 0 {
                snapshot.native[i]
            } else {
                snapshot.native[i].wrapping_sub(1)
            };
            symbolize_native_addr(resolve_addr, cache)
        })
        .collect();

    let python_outer_to_inner: Vec<CallFrame> = snapshot.py[..plen]
        .iter()
        .map(|&key| resolve_py_call_frame(key))
        .collect();

    let frames = merge_python_native_stacks(&python_outer_to_inner, &native_leaf_to_root);
    ParsedStacks {
        tid: snapshot.tid,
        source: snapshot.source,
        flags: snapshot.flags,
        frames,
    }
}

/// Parse with a multi-slot `(tid, seq)` view cache (HTTP / dynamic reuse).
pub fn parse_snapshot_cached(
    snapshot: &StackSnapshot,
    seq: u64,
    cache: &mut HashMap<usize, CallFrame>,
) -> ParsedStacks {
    if seq != 0 {
        if let Ok(guard) = PARSED_VIEW_CACHE.lock() {
            if let Some(parsed) = guard.get(snapshot.tid, seq) {
                metrics::inc_parse_cache_hit();
                return parsed.clone();
            }
        }
    }
    let parsed = parse_snapshot(snapshot, cache);
    if seq != 0 {
        if let Ok(mut guard) = PARSED_VIEW_CACHE.lock() {
            guard.insert(snapshot.tid, seq, parsed.clone());
        }
    }
    parsed
}

impl StackSnapshot {
    /// Lazy materialization of merged [`CallFrame`]s (root → leaf).
    pub fn parse(&self, cache: &mut HashMap<usize, CallFrame>) -> ParsedStacks {
        parse_snapshot(self, cache)
    }

    /// Like [`Self::parse`], but reuse a prior result for the same `(tid, seq)`.
    pub fn parse_cached(&self, seq: u64, cache: &mut HashMap<usize, CallFrame>) -> ParsedStacks {
        parse_snapshot_cached(self, seq, cache)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::stacktrace::snapshot::StackSource;

    fn c(func: &str) -> CallFrame {
        CallFrame::CFrame {
            ip: "0x1".into(),
            file: String::new(),
            func: func.into(),
            lineno: 0,
            lang: Some("cpp".into()),
        }
    }

    #[test]
    fn torn_py_yields_empty_parsed() {
        let mut s = StackSnapshot::from_parts(
            1,
            StackSource::Sigprof,
            &[0x1000],
            &[0x2000],
            StackFlags::PY_TORN,
        );
        s.flags.insert(StackFlags::PY_TORN);
        let mut cache = HashMap::new();
        let parsed = s.parse(&mut cache);
        assert!(parsed.is_empty());
        assert_eq!(parsed.source, StackSource::Sigprof);
    }

    #[test]
    fn parse_cache_hits_on_same_tid_seq() {
        use crate::features::stacktrace::metrics;
        let before_hits = metrics::parse_cache_hits();

        let s = StackSnapshot::from_parts(3, StackSource::Sigprof, &[], &[], StackFlags::PY_ABSENT);
        let mut cache = HashMap::new();
        let a = parse_snapshot_cached(&s, 42, &mut cache);
        let b = parse_snapshot_cached(&s, 42, &mut cache);
        assert_eq!(a.tid, b.tid);
        assert_eq!(a.frames.len(), b.frames.len());
        assert!(metrics::parse_cache_hits() > before_hits);
    }

    #[test]
    fn parse_cache_holds_distinct_tid_seq() {
        use crate::features::stacktrace::metrics;
        let mut cache = HashMap::new();
        let a = StackSnapshot::from_parts(1, StackSource::Sigprof, &[], &[], StackFlags::PY_ABSENT);
        let b = StackSnapshot::from_parts(2, StackSource::Sigprof, &[], &[], StackFlags::PY_ABSENT);
        let _ = parse_snapshot_cached(&a, 10, &mut cache);
        let _ = parse_snapshot_cached(&b, 11, &mut cache);
        // Both slots remain addressable (not a single global overwrite).
        let before = metrics::parse_cache_hits();
        let _ = parse_snapshot_cached(&a, 10, &mut cache);
        let _ = parse_snapshot_cached(&b, 11, &mut cache);
        assert!(metrics::parse_cache_hits() >= before + 2);
    }

    #[test]
    fn native_only_preserves_source_and_tid() {
        // Addresses won't resolve meaningfully; merge still returns frames or empty.
        let s = StackSnapshot::from_parts(
            9,
            StackSource::SyncWalk,
            &[0x10, 0x20],
            &[],
            StackFlags::empty(),
        );
        let mut cache = HashMap::new();
        // Pre-seed cache so merge sees known symbols (leaf → root input).
        cache.insert(0x10, c("leaf"));
        cache.insert(0x1f, c("root")); // wrapping_sub(1) on second frame
        let parsed = parse_snapshot(&s, &mut cache);
        assert_eq!(parsed.tid, 9);
        assert_eq!(parsed.source, StackSource::SyncWalk);
        assert!(!parsed.frames.is_empty());
        // root → leaf after merge
        assert_eq!(
            match &parsed.frames[0] {
                CallFrame::CFrame { func, .. } => func.as_str(),
                _ => "",
            },
            "root"
        );
    }
}
