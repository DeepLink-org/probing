//! Shared helpers: unique names/paths, temp dirs, and hot-ring population.

use std::io;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use probing_memtable::memc::{ColdStore, Compactor, CompactorConfig};
use probing_memtable::{DType, MemTable};

use crate::cli::bench::args::RingArgs;
use crate::cli::bench::workload::{RowGen, WorkloadSpec};

/// How to attach to an already-created shared table (used by multi-handle
/// runners). Heap is excluded because it cannot be shared.
#[derive(Clone)]
pub enum Attach {
    Shm(String),
    File(PathBuf),
}

impl Attach {
    pub fn open(&self) -> io::Result<MemTable> {
        match self {
            Attach::Shm(name) => MemTable::open_shm(name),
            Attach::File(path) => MemTable::open_file(path),
        }
    }

    /// Serialize for passing to a child process (`shm:<name>` / `file:<path>`).
    pub fn encode(&self) -> String {
        match self {
            Attach::Shm(name) => format!("shm:{name}"),
            Attach::File(path) => format!("file:{}", path.display()),
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        if let Some(name) = s.strip_prefix("shm:") {
            Ok(Attach::Shm(name.to_string()))
        } else if let Some(path) = s.strip_prefix("file:") {
            Ok(Attach::File(PathBuf::from(path)))
        } else {
            anyhow::bail!("invalid attach descriptor: {s}")
        }
    }
}

/// Scan all resident rows of `table` once through the cursor, folding values
/// into a sink. Returns `(value_sink, row_count)`.
pub fn scan_all(table: &MemTable, dtypes: &[DType]) -> (u64, u64) {
    let mut sink = 0u64;
    let mut rows = 0u64;
    for chunk in table.chunks_logical() {
        for row in table.rows(chunk) {
            let mut c = row.cursor();
            for dt in dtypes {
                match dt {
                    DType::U8 => sink = sink.wrapping_add(c.next_u8() as u64),
                    DType::U32 => sink = sink.wrapping_add(c.next_u32() as u64),
                    DType::I32 => sink = sink.wrapping_add(c.next_i32() as u64),
                    DType::I64 => sink = sink.wrapping_add(c.next_i64() as u64),
                    DType::U64 => sink = sink.wrapping_add(c.next_u64()),
                    DType::F32 => sink = sink.wrapping_add(c.next_f32().to_bits() as u64),
                    DType::F64 => sink = sink.wrapping_add(c.next_f64().to_bits()),
                    DType::Str => sink = sink.wrapping_add(c.next_str().len() as u64),
                    DType::Bytes => sink = sink.wrapping_add(c.next_bytes().len() as u64),
                }
            }
            rows += 1;
        }
    }
    (sink, rows)
}

/// A process-and-time unique token for naming temp files / shm objects.
pub fn unique_token() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{}-{}", std::process::id(), nanos % 1_000_000)
}

/// A temp file path (not created).
pub fn temp_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!("probing-bench-{label}-{}.memt", unique_token()))
}

/// A temp directory path (created).
pub fn temp_dir(label: &str) -> Result<PathBuf> {
    let dir = std::env::temp_dir().join(format!("probing-bench-{label}-{}", unique_token()));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// POSIX shm name (short enough for macOS' 31-byte cap).
pub fn shm_name() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("/pb{}", nanos % 1_000_000_000)
}

/// Fill a hot table with `rows` rows via the concurrency-safe `push_row`
/// path, returning the number of rows written. Used by scan/compact/cold
/// builders where ingest speed is not the measured quantity.
pub fn populate(table: &mut MemTable, spec: &WorkloadSpec, rows: u64, seed: u64) -> u64 {
    let mut gen = RowGen::new(spec.clone(), seed, 0);
    let mut scratch: Vec<f64> = Vec::new();
    for _ in 0..rows {
        let values = gen.values(&mut scratch);
        table.push_row_unchecked(&values);
    }
    rows
}

/// Ingest `rows` rows and compact them into MEMC segments under `dir`,
/// interleaving drains so the ring never overwrites undrained chunks.
/// Returns rows actually drained to cold.
pub fn build_cold(
    dir: &std::path::Path,
    spec: &WorkloadSpec,
    ring: &RingArgs,
    rows: u64,
    target_mb: u64,
    seed: u64,
) -> Result<u64> {
    let row_bytes = spec.approx_row_bytes() as u64;
    let mut table = MemTable::new(&spec.schema(), ring.chunk_size, ring.chunks);
    let store = ColdStore::open(dir)?;
    let config = CompactorConfig {
        target_segment_bytes: target_mb * 1024 * 1024,
        max_segment_age: Duration::from_secs(3600),
        poll_interval: Duration::from_millis(1),
        max_total_bytes: None,
        ttl: None,
    };
    let mut compactor = Compactor::new(store, config);

    let rows_per_chunk = ((ring.chunk_size as u64).saturating_sub(40)) / (row_bytes + 4).max(1);
    let batch = (rows_per_chunk * (ring.chunks as u64 / 2).max(1)).max(1);

    let mut gen = RowGen::new(spec.clone(), seed, 0);
    let mut scratch: Vec<f64> = Vec::new();
    let mut ingested = 0u64;
    let mut drained = 0u64;
    while ingested < rows {
        let n = batch.min(rows - ingested);
        for _ in 0..n {
            let values = gen.values(&mut scratch);
            table.push_row_unchecked(&values);
        }
        ingested += n;
        drained += compactor.drain_view("bench", &table.view())? as u64;
    }
    loop {
        let n = compactor.drain_view("bench", &table.view())? as u64;
        drained += n;
        if n == 0 {
            break;
        }
    }
    compactor.flush()?;
    Ok(drained)
}
