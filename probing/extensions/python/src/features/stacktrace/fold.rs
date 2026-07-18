//! [`ParsedStacks`] / [`StackSnapshot`] → [`FoldedStacks`] for flamegraphs.
//!
//! Aggregation and distributed merge operate on this layer. Prefer
//! [`fold_snapshot`] on the sampling consumer (avoids exposing intermediate
//! frames to callers that only need folded text).

use std::collections::{BTreeSet, HashMap};

use probing_proto::prelude::CallFrame;

use crate::features::stacktrace::capture::thread_name;
use crate::features::stacktrace::merge::merged_frames_to_folded_segments;
use crate::features::stacktrace::metrics;
use crate::features::stacktrace::parse::{parse_snapshot, ParsedStacks};
use crate::features::stacktrace::snapshot::{StackSnapshot, StackSource};

/// Options for projecting frames into folded segments.
#[derive(Clone, Debug)]
pub struct FoldOptions {
    /// Prefix `thread-{tid}` / `thread-{tid} (name)`.
    pub thread_prefix: bool,
    /// Strip interpreter / stdlib bootstrap (for cross-rank merge).
    pub canonicalize: bool,
    /// Sample count carried on this folded stack (usually 1 before aggregate).
    pub count: u64,
}

impl Default for FoldOptions {
    fn default() -> Self {
        Self {
            thread_prefix: true,
            canonicalize: true,
            count: 1,
        }
    }
}

/// Flamegraph-oriented stack: root → leaf segments + weight + optional ranks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FoldedStacks {
    pub tid: u64,
    pub source: StackSource,
    pub segments: Vec<String>,
    pub count: u64,
    pub ranks: Vec<i32>,
}

impl FoldedStacks {
    pub fn is_empty(&self) -> bool {
        self.segments.is_empty() || self.count == 0
    }

    /// `"path count"` line used by classic folded-file consumers.
    pub fn to_folded_line(&self) -> String {
        if self.segments.is_empty() {
            return String::new();
        }
        format!("{} {}", self.segments.join(";"), self.count)
    }

    /// Path string without count (aggregation key).
    pub fn path_key(&self) -> String {
        self.segments.join(";")
    }

    /// Keep only `[py]` segments.
    pub fn python_only(&self) -> Option<Self> {
        let segments: Vec<String> = self
            .segments
            .iter()
            .filter(|s| s.starts_with("[py]"))
            .cloned()
            .collect();
        if segments.is_empty() {
            return None;
        }
        Some(Self {
            tid: self.tid,
            source: self.source,
            segments,
            count: self.count,
            ranks: self.ranks.clone(),
        })
    }
}

fn thread_prefix_segment(tid: u64) -> String {
    match thread_name(tid) {
        Some(name) => format!("thread-{tid} ({name})"),
        None => format!("thread-{tid}"),
    }
}

/// Fold already-parsed frames.
pub fn fold_parsed(parsed: &ParsedStacks, opts: &FoldOptions) -> FoldedStacks {
    metrics::inc_fold_call();
    let mut segments = if opts.canonicalize {
        merged_frames_to_folded_segments(&parsed.frames)
    } else {
        raw_segments_from_frames(&parsed.frames)
    };
    if opts.thread_prefix && !segments.is_empty() {
        let mut with_thread = vec![thread_prefix_segment(parsed.tid)];
        with_thread.append(&mut segments);
        segments = with_thread;
    }
    FoldedStacks {
        tid: parsed.tid,
        source: parsed.source,
        segments,
        count: opts.count.max(1),
        ranks: Vec::new(),
    }
}

fn raw_segments_from_frames(frames: &[CallFrame]) -> Vec<String> {
    frames
        .iter()
        .map(|frame| match frame {
            CallFrame::PyFrame {
                file, func, lineno, ..
            } => {
                let base = file.rsplit(['/', '\\']).next().unwrap_or(file);
                format!("[py] {func} ({base}:{lineno})")
            }
            CallFrame::CFrame { func, .. } => func.clone(),
        })
        .collect()
}

/// Snapshot → folded in one step (sampling / export fast path).
///
/// Logically equivalent to `parse` then `fold_parsed` with the same options.
pub fn fold_snapshot(
    snapshot: &StackSnapshot,
    cache: &mut HashMap<usize, CallFrame>,
    opts: &FoldOptions,
) -> FoldedStacks {
    let parsed = parse_snapshot(snapshot, cache);
    fold_parsed(&parsed, opts)
}

impl StackSnapshot {
    pub fn fold(&self, cache: &mut HashMap<usize, CallFrame>, opts: &FoldOptions) -> FoldedStacks {
        fold_snapshot(self, cache, opts)
    }
}

impl ParsedStacks {
    pub fn fold(&self, opts: &FoldOptions) -> FoldedStacks {
        fold_parsed(self, opts)
    }
}

/// Merge folded stacks across ranks: sum counts, union ranks, canonicalize paths.
///
/// Input lines are classic `"path count"` strings (optionally with thread prefixes).
pub fn merge_folded_attributed(sets: &[(Option<i32>, Vec<FoldedStacks>)]) -> Vec<FoldedStacks> {
    let mut counts: HashMap<String, u64> = HashMap::new();
    let mut ranks: HashMap<String, BTreeSet<i32>> = HashMap::new();
    let mut sources: HashMap<String, StackSource> = HashMap::new();

    for (rank, stacks) in sets {
        for stack in stacks {
            let mut segs = stack.segments.clone();
            while segs.first().is_some_and(|s| s == "all") {
                segs.remove(0);
            }
            while segs.first().is_some_and(|s| is_thread_folded_segment(s)) {
                segs.remove(0);
            }
            let segs = crate::features::stacktrace::merge::canonicalize_folded_segments(&segs);
            if segs.is_empty() {
                continue;
            }
            let key = segs.join(";");
            *counts.entry(key.clone()).or_insert(0) += stack.count.max(1);
            sources.entry(key.clone()).or_insert(stack.source);
            if let Some(r) = rank {
                ranks.entry(key).or_default().insert(*r);
            }
        }
    }

    let mut merged: Vec<FoldedStacks> = counts
        .into_iter()
        .map(|(key, count)| {
            let segments: Vec<String> = key.split(';').map(str::to_string).collect();
            let ranks = ranks
                .remove(&key)
                .map(|s| s.into_iter().collect())
                .unwrap_or_default();
            let source = sources.remove(&key).unwrap_or(StackSource::Unknown);
            FoldedStacks {
                tid: 0,
                source,
                segments,
                count,
                ranks,
            }
        })
        .collect();
    merged.sort_by(|a, b| a.segments.cmp(&b.segments));
    merged
}

/// Parse classic folded lines into [`FoldedStacks`] (no thread metadata).
pub fn folded_from_line(line: &str, source: StackSource) -> Option<FoldedStacks> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let (stack, count_str) = line.rsplit_once(' ')?;
    let count = count_str.parse::<u64>().ok()?;
    if count == 0 {
        return None;
    }
    let segments: Vec<String> = stack
        .split(';')
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect();
    if segments.is_empty() {
        return None;
    }
    Some(FoldedStacks {
        tid: 0,
        source,
        segments,
        count,
        ranks: Vec::new(),
    })
}

fn is_thread_folded_segment(seg: &str) -> bool {
    let Some(rest) = seg.strip_prefix("thread-") else {
        return false;
    };
    let digit_len = rest.chars().take_while(|c| c.is_ascii_digit()).count();
    digit_len > 0
        && (rest.len() == digit_len
            || matches!(rest.as_bytes().get(digit_len), Some(b' ') | Some(b'(')))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::stacktrace::parse::parse_snapshot;
    use crate::features::stacktrace::snapshot::StackSource;

    #[test]
    fn fold_parsed_adds_thread_prefix() {
        let parsed = ParsedStacks {
            tid: 42,
            source: StackSource::Sigprof,
            flags: Default::default(),
            frames: vec![CallFrame::CFrame {
                ip: "0x1".into(),
                file: String::new(),
                func: "foo".into(),
                lineno: 0,
                lang: None,
            }],
        };
        let folded = fold_parsed(
            &parsed,
            &FoldOptions {
                thread_prefix: true,
                canonicalize: false,
                count: 3,
            },
        );
        assert_eq!(folded.count, 3);
        assert!(folded.segments[0].starts_with("thread-42"));
        assert_eq!(folded.segments.last().map(String::as_str), Some("foo"));
        assert_eq!(folded.to_folded_line().rsplit_once(' ').unwrap().1, "3");
    }

    #[test]
    fn merge_folded_sums_counts_and_ranks() {
        let a = FoldedStacks {
            tid: 1,
            source: StackSource::Sigprof,
            segments: vec!["[py] main (a.py:1)".into(), "[py] train (a.py:2)".into()],
            count: 2,
            ranks: vec![],
        };
        let b = FoldedStacks {
            tid: 2,
            source: StackSource::Sigprof,
            segments: vec![
                "thread-9".into(),
                "[py] main (a.py:1)".into(),
                "[py] train (a.py:2)".into(),
            ],
            count: 3,
            ranks: vec![],
        };
        let merged = merge_folded_attributed(&[(Some(0), vec![a]), (Some(1), vec![b])]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].count, 5);
        assert_eq!(merged[0].ranks, vec![0, 1]);
    }

    #[test]
    fn fold_snapshot_matches_parse_then_fold() {
        use crate::features::stacktrace::snapshot::{StackFlags, StackSnapshot, StackSource};
        let snap =
            StackSnapshot::from_parts(7, StackSource::Sigprof, &[], &[], StackFlags::PY_ABSENT);
        let mut cache_a = HashMap::new();
        let mut cache_b = HashMap::new();
        let opts = FoldOptions {
            thread_prefix: false,
            canonicalize: true,
            count: 1,
        };
        let via_fast = fold_snapshot(&snap, &mut cache_a, &opts);
        let via_parse = parse_snapshot(&snap, &mut cache_b).fold(&opts);
        assert_eq!(via_fast, via_parse);
    }

    #[test]
    fn python_only_strips_native() {
        let f = FoldedStacks {
            tid: 0,
            source: StackSource::Sigprof,
            segments: vec![
                "[py] a (a.py:1)".into(),
                "native_fn".into(),
                "[py] b (b.py:2)".into(),
            ],
            count: 1,
            ranks: vec![0],
        };
        let py = f.python_only().unwrap();
        assert_eq!(
            py.segments,
            vec!["[py] a (a.py:1)".to_string(), "[py] b (b.py:2)".to_string()]
        );
        assert_eq!(py.ranks, vec![0]);
    }
}
