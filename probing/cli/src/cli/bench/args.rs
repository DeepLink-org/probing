//! Argument structs for the `bench` subcommands (clap derive).

use std::path::PathBuf;

use clap::{Args, ValueEnum};

use super::workload::{SchemaKind, WorkloadSpec};

/// Storage backend under test.
#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// Process-private heap buffer (no cross-handle sharing).
    Heap,
    /// POSIX shared memory (`shm_open`).
    Shm,
    /// mmap'd regular file at an explicit path.
    File,
    /// Discoverable mmap'd file under the data dir (SQL-visible).
    Shared,
}

/// Streaming row writer vs. value-vector `push_row`.
#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriterMode {
    /// `push_row` — auto-advance on chunk full; allocates a value row.
    Push,
    /// `RowWriter` streaming fast path (zero per-row allocation).
    Streaming,
}

/// Schema / row-shape options shared by every subcommand.
#[derive(Args, Debug, Clone)]
pub struct SchemaArgs {
    /// Built-in column layout.
    #[arg(long, value_enum, default_value = "metrics")]
    pub schema: SchemaKind,

    /// Number of f64 columns for `--schema wide`.
    #[arg(long, default_value_t = 16)]
    pub wide_cols: usize,

    /// Byte length of the `msg` payload for `--schema logs`.
    #[arg(long, default_value_t = 32)]
    pub str_len: usize,
}

impl SchemaArgs {
    pub fn spec(&self) -> WorkloadSpec {
        WorkloadSpec {
            kind: self.schema,
            wide_cols: self.wide_cols.max(1),
            str_len: self.str_len,
        }
    }
}

/// Ring geometry shared by subcommands that build a hot table.
#[derive(Args, Debug, Clone)]
pub struct RingArgs {
    /// Bytes per ring chunk.
    #[arg(long, default_value_t = 256 * 1024)]
    pub chunk_size: u32,

    /// Number of ring chunks (slots).
    #[arg(long, default_value_t = 64)]
    pub chunks: u32,
}

#[derive(Args, Debug, Clone)]
pub struct WriteArgs {
    #[command(flatten)]
    pub schema: SchemaArgs,
    #[command(flatten)]
    pub ring: RingArgs,

    /// Storage backend.
    #[arg(long, value_enum, default_value = "heap")]
    pub backend: Backend,

    /// Total rows to write (across all threads).
    #[arg(long, default_value_t = 1_000_000)]
    pub rows: u64,

    /// Concurrent writer threads. Only valid with `--backend heap`, where each
    /// thread gets its own independent table. Shared backends are single-writer.
    #[arg(long, default_value_t = 1)]
    pub threads: usize,

    /// Writer API to exercise. `streaming` is the zero-allocation single-row
    /// fast path (no per-row value vector); `push` allocates a value row.
    #[arg(long, value_enum, default_value = "streaming")]
    pub writer: WriterMode,

    /// File path for `--backend file` (defaults to a temp file).
    #[arg(long)]
    pub path: Option<PathBuf>,

    /// Record a per-row latency histogram (adds measurable overhead).
    #[arg(long)]
    pub latency: bool,

    /// Warm-up rows per thread, excluded from measurement.
    #[arg(long, default_value_t = 0)]
    pub warmup: u64,
}

#[derive(Args, Debug, Clone)]
pub struct ScanArgs {
    #[command(flatten)]
    pub schema: SchemaArgs,
    #[command(flatten)]
    pub ring: RingArgs,

    /// Rows to pre-populate before scanning.
    #[arg(long, default_value_t = 1_000_000)]
    pub rows: u64,

    /// Number of full scan passes to time.
    #[arg(long, default_value_t = 5)]
    pub iters: usize,
}

#[derive(Args, Debug, Clone)]
pub struct CompactArgs {
    #[command(flatten)]
    pub schema: SchemaArgs,
    #[command(flatten)]
    pub ring: RingArgs,

    /// Rows to ingest and compact.
    #[arg(long, default_value_t = 2_000_000)]
    pub rows: u64,

    /// Segment roll size in MiB (`target_segment_bytes`).
    #[arg(long, default_value_t = 8)]
    pub target_mb: u64,

    /// Cold directory (defaults to a temp dir; removed on exit unless --keep).
    #[arg(long)]
    pub dir: Option<PathBuf>,

    /// Keep the cold directory after the run.
    #[arg(long)]
    pub keep: bool,
}

#[derive(Args, Debug, Clone)]
pub struct ColdscanArgs {
    #[command(flatten)]
    pub schema: SchemaArgs,
    #[command(flatten)]
    pub ring: RingArgs,

    /// Read an existing cold directory instead of building one.
    #[arg(long)]
    pub dir: Option<PathBuf>,

    /// Rows to ingest when building a cold store (ignored with --dir).
    #[arg(long, default_value_t = 2_000_000)]
    pub rows: u64,

    /// Segment roll size in MiB when building (ignored with --dir).
    #[arg(long, default_value_t = 8)]
    pub target_mb: u64,

    /// Number of full read passes to time.
    #[arg(long, default_value_t = 3)]
    pub iters: usize,
}

#[derive(Args, Debug, Clone)]
pub struct MixedArgs {
    #[command(flatten)]
    pub schema: SchemaArgs,
    #[command(flatten)]
    pub ring: RingArgs,

    /// Shared backend for the pipeline.
    #[arg(long, value_enum, default_value = "shared")]
    pub backend: Backend,

    /// Writer threads. MEMT is single-writer; must be 1.
    #[arg(long, default_value_t = 1)]
    pub writers: usize,

    /// Concurrent reader (scan) threads.
    #[arg(long, default_value_t = 1)]
    pub readers: usize,

    /// Run duration in seconds.
    #[arg(long, default_value_t = 10)]
    pub duration: u64,

    /// Disable the background compactor (hot-only pipeline).
    #[arg(long)]
    pub no_compact: bool,

    /// Segment roll size in MiB.
    #[arg(long, default_value_t = 8)]
    pub target_mb: u64,

    /// Cold-store byte budget in MiB (eviction trigger).
    #[arg(long)]
    pub max_total_mb: Option<u64>,

    /// Cold-store TTL in seconds.
    #[arg(long)]
    pub ttl_secs: Option<u64>,
}

#[derive(Args, Debug, Clone)]
pub struct MpArgs {
    #[command(flatten)]
    pub schema: SchemaArgs,
    #[command(flatten)]
    pub ring: RingArgs,

    /// Shared backend (must be cross-process: shm/file/shared).
    #[arg(long, value_enum, default_value = "shared")]
    pub backend: Backend,

    /// Writer processes. MEMT is single-writer; must be 1.
    #[arg(long, default_value_t = 1)]
    pub writers: usize,

    /// Number of reader processes.
    #[arg(long, default_value_t = 2)]
    pub readers: usize,

    /// Measurement window in seconds (the soak is time-driven, not row-driven).
    #[arg(long, default_value_t = 10)]
    pub duration: u64,
}
