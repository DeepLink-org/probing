//! Unified stack tracing for Python / C++ / Rust.
//!
//! # Pipeline
//!
//! ```text
//! fill adapters → StackSnapshot → parse → ParsedStacks → fold → FoldedStacks
//! ```
//!
//! | Type | Role |
//! |------|------|
//! | [`StackSnapshot`] | Signal-safe POD (leaf→root native, outer→inner py keys) |
//! | [`ParsedStacks`] | Demangled + merged frames (root→leaf) |
//! | [`FoldedStacks`] | Flamegraph / distributed aggregation |
//!
//! | Module | Role |
//! |--------|------|
//! | [`snapshot`] | Document types + `StackSource` / flags |
//! | [`compact`] | Heap-sized payload for sampler buckets |
//! | [`fingerprint`] | Aggregation key (`tid` + PCs + py keys + flags) |
//! | [`parse`] | Snapshot → ParsedStacks (+ view cache) |
//! | [`fold`] | Parsed/Snapshot → FoldedStacks + merge |
//! | [`metrics`] | Drop / fingerprint / parse / fold counters |
//! | [`merge`] | Python ⊕ native splice + canonicalize |
//! | [`capture`] | Registry, intern, signal fill (no parse/fold logic) |
//! | [`spy`] | CPython ABI / TLS (vendored py-spy; do not casually edit) |
//! | [`tracers`] | vm / pprof / dynamic fill policies |

pub mod capture;
pub mod compact;
pub mod fingerprint;
pub mod fold;
pub mod merge;
pub mod metrics;
pub mod parse;
pub mod snapshot;
pub mod spy;
pub mod tracers;

pub use fold::{fold_parsed, fold_snapshot, merge_folded_attributed, FoldOptions, FoldedStacks};
pub use merge::{
    canonicalize_folded_segments, demangle_native_symbol, merge_python_native_stacks,
    merged_frames_to_folded_segments,
};
pub use parse::{parse_snapshot, ParsedStacks};
pub use snapshot::{RawStackSnapshot, StackFlags, StackSnapshot, StackSource, MAX_NATIVE, MAX_PY};
pub(crate) use tracers::dynamic::is_backtrace_busy;
pub use tracers::dynamic::{SignalTracer, StackTracer};
pub use tracers::vm;
