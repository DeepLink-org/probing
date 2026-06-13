//! [`Compactor`]: the **roller** that drains sealed hot-ring chunks into
//! cold MEMC segments, bounding segment size to prevent fragmentation.
//!
//! ## Why a roller
//!
//! The MEMC format and [`ColdStore`] give us immutable segments and
//! oldest-first eviction, but *nothing decides when to seal a segment and
//! start a fresh one*. Without that policy you either seal every page
//! (a blizzard of tiny files) or never seal (one unbounded file). The
//! compactor closes that gap with a size-or-time roll policy:
//!
//! ```text
//! after each appended page:
//!     size_bytes() >= target_segment_bytes   → seal + roll
//! on every poll tick:
//!     open segment older than max_segment_age → seal + roll (low-rate tables)
//! on shutdown / flush:
//!     seal the open segment unconditionally   → bounded tail file
//! ```
//!
//! A busy process emits a steady stream of ~`target`-sized files; an idle
//! one keeps appending to a single open segment until the age window or
//! shutdown, so neither extreme fragments the directory.
//!
//! ## Multi-table
//!
//! One [`Compactor`] feeds **one** [`ColdStore`] from **many** hot tables.
//! Pages from every table share the same segment files (each carries its
//! `table_id`), so adding tables grows pages, not files or directories.
//!
//! ## Concurrency
//!
//! The hot table is written by the application; the compactor only ever
//! *reads* it. For shared/file-backed tables the compactor opens its own
//! read handle to the same mapping and relies on the ring's lock-free
//! `Acquire`/`Release` chunk protocol: it drains only `Sealed` chunks and
//! re-checks the chunk generation after transposing, discarding a page if
//! the ring recycled the chunk mid-read. The still-open `Writing` chunk is
//! left to the hot tier until it seals.

use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::codec::{ColumnBuilder, ColumnData};
use super::layout::SOURCE_CHUNK_NONE;
use super::reader::SegmentReader;
use super::store::ColdStore;
use super::writer::SegmentWriter;
use crate::layout::ChunkState;
use crate::memtable::{MemTable, MemTableView};
use crate::schema::{DType, Value};

/// Roll/retention policy for a [`Compactor`].
#[derive(Debug, Clone)]
pub struct CompactorConfig {
    /// Seal the open segment and start a new one once it reaches this many
    /// bytes. Bounds individual file size; the main fragmentation knob.
    pub target_segment_bytes: u64,
    /// Also seal an open segment this old, so low-rate tables don't sit
    /// unsealed (and unqueryable through the cold footer) indefinitely.
    pub max_segment_age: Duration,
    /// How long the background thread sleeps between drain passes.
    pub poll_interval: Duration,
    /// Cold-store byte budget; oldest segments are evicted past it.
    pub max_total_bytes: Option<u64>,
    /// Cold-store TTL; segments older than this are evicted.
    pub ttl: Option<Duration>,
}

impl Default for CompactorConfig {
    fn default() -> Self {
        Self {
            target_segment_bytes: 64 * 1024 * 1024,
            max_segment_age: Duration::from_secs(300),
            poll_interval: Duration::from_millis(500),
            max_total_bytes: None,
            ttl: None,
        }
    }
}

/// Per-table draining bookkeeping.
struct TableProgress {
    /// Last drained generation per chunk index (parallel to the hot ring).
    /// A chunk is re-drained only when its generation advances past this.
    drained_gen: Vec<u64>,
    /// This table's id inside the *current* open segment, if registered.
    /// Reset to `None` on every roll (table ids are segment-local).
    seg_table_id: Option<u32>,
}

/// Drains sealed hot chunks into size-bounded cold segments.
///
/// Usable synchronously (call [`drain_view`](Self::drain_view) yourself) or
/// as a background thread via [`spawn`](Self::spawn).
pub struct Compactor {
    store: ColdStore,
    config: CompactorConfig,
    current: Option<SegmentWriter>,
    opened_at: Instant,
    tables: HashMap<String, TableProgress>,
    /// Per-(table, chunk) drain watermark recovered from existing cold
    /// segments by [`prime_from_cold`](Self::prime_from_cold); merged into a
    /// table's `drained_gen` the first time it is seen, so a restart over a
    /// persistent cold dir does not re-compact already-persisted chunks.
    seed: HashMap<String, HashMap<usize, u64>>,
}

impl Compactor {
    pub fn new(store: ColdStore, config: CompactorConfig) -> Self {
        Self {
            store,
            config,
            current: None,
            opened_at: Instant::now(),
            tables: HashMap::new(),
            seed: HashMap::new(),
        }
    }

    /// Rebuild per-(table, chunk) drain watermarks from the cold segments
    /// already on disk, so draining is **exactly-once across restarts** when
    /// the cold dir persists. Call once after [`new`](Self::new), before any
    /// `drain_view`. Each cold page records the hot-ring `(source_gen,
    /// source_chunk)` it came from; we keep the max generation per chunk.
    pub fn prime_from_cold(&mut self) -> io::Result<()> {
        for path in self.store.segment_paths() {
            let Ok(reader) = SegmentReader::open(&path) else {
                continue; // unreadable/foreign file: skip, never fail priming
            };
            for page in reader.pages() {
                if page.source_chunk == SOURCE_CHUNK_NONE {
                    continue;
                }
                let Some(def) = reader.table_def(page.table_id) else {
                    continue;
                };
                let slot = self
                    .seed
                    .entry(def.name.clone())
                    .or_default()
                    .entry(page.source_chunk as usize)
                    .or_insert(0);
                *slot = (*slot).max(page.source_gen);
            }
        }
        Ok(())
    }

    pub fn config(&self) -> &CompactorConfig {
        &self.config
    }

    /// Bytes written to the currently open segment (0 if none).
    pub fn current_segment_bytes(&self) -> u64 {
        self.current.as_ref().map(|w| w.size_bytes()).unwrap_or(0)
    }

    /// Cold-store capacity snapshot.
    pub fn stats(&self) -> super::store::ColdStats {
        self.store.stats()
    }

    /// Drain every newly-sealed chunk of `view` (a read handle to a hot
    /// table named `name`) into cold pages, rolling segments by size as it
    /// goes. Returns the number of rows compacted this call.
    pub fn drain_view(&mut self, name: &str, view: &MemTableView) -> io::Result<usize> {
        let cols: Vec<(String, DType)> = view
            .schema()
            .cols
            .iter()
            .map(|c| (c.name.clone(), c.dtype))
            .collect();
        let num_chunks = view.num_chunks();

        if !self.tables.contains_key(name) {
            let mut drained_gen = vec![0u64; num_chunks];
            if let Some(seeds) = self.seed.get(name) {
                for (&chunk, &gen) in seeds {
                    if chunk < drained_gen.len() {
                        drained_gen[chunk] = drained_gen[chunk].max(gen);
                    }
                }
            }
            self.tables.insert(
                name.to_string(),
                TableProgress {
                    drained_gen,
                    seg_table_id: None,
                },
            );
        }
        let prog = self.tables.get_mut(name).unwrap();
        if prog.drained_gen.len() != num_chunks {
            prog.drained_gen.resize(num_chunks, 0);
        }

        let sealed = ChunkState::Sealed as u32;
        let mut total_rows = 0usize;

        for chunk in view.chunks_logical() {
            if view.chunk_state(chunk) != sealed {
                continue;
            }
            let gen = view.chunk_generation(chunk);
            let already = self.tables[name].drained_gen[chunk];
            if gen == 0 || gen <= already {
                continue;
            }

            let (gen_read, columns) = match transpose_chunk(view, chunk, &cols) {
                Some(x) => x,
                None => continue, // recycled mid-read; try again next pass
            };
            let rows = columns.first().map(|c| c.len()).unwrap_or(0);
            if rows == 0 {
                self.tables.get_mut(name).unwrap().drained_gen[chunk] = gen_read;
                continue;
            }

            self.ensure_segment()?;
            let table_id = self.register_if_needed(name, &cols)?;
            self.current
                .as_mut()
                .expect("segment open")
                .append_page(table_id, &columns, gen_read, chunk as u32)?;
            self.tables.get_mut(name).unwrap().drained_gen[chunk] = gen_read;
            total_rows += rows;

            self.maybe_roll_on_size()?;
        }
        Ok(total_rows)
    }

    /// Seal the open segment if it has grown past `target_segment_bytes`.
    fn maybe_roll_on_size(&mut self) -> io::Result<Option<PathBuf>> {
        let over = self
            .current
            .as_ref()
            .is_some_and(|w| w.size_bytes() >= self.config.target_segment_bytes);
        if over {
            self.roll()
        } else {
            Ok(None)
        }
    }

    /// Seal the open segment if it is older than `max_segment_age` and holds
    /// at least one page. Call this periodically (the background loop does).
    pub fn maybe_roll_on_age(&mut self) -> io::Result<Option<PathBuf>> {
        let aged = self.current.as_ref().is_some_and(|w| w.page_count() > 0)
            && self.opened_at.elapsed() >= self.config.max_segment_age;
        if aged {
            self.roll()
        } else {
            Ok(None)
        }
    }

    /// Seal the current segment and clear the open slot. An open segment
    /// with no pages is removed instead of sealed, so an age-triggered roll
    /// on an empty writer never leaves a stub file. Returns the sealed path.
    pub fn roll(&mut self) -> io::Result<Option<PathBuf>> {
        let Some(w) = self.current.take() else {
            return Ok(None);
        };
        for p in self.tables.values_mut() {
            p.seg_table_id = None;
        }
        if w.page_count() == 0 {
            let path = w.path().to_path_buf();
            drop(w);
            let _ = std::fs::remove_file(&path);
            return Ok(None);
        }
        Ok(Some(w.seal()?))
    }

    /// Seal whatever is open (shutdown / explicit checkpoint).
    pub fn flush(&mut self) -> io::Result<Option<PathBuf>> {
        self.roll()
    }

    /// Apply the cold-store byte/TTL budget, deleting oldest segments.
    pub fn enforce(&self) -> Vec<PathBuf> {
        self.store
            .enforce_limits(self.config.max_total_bytes, self.config.ttl)
    }

    fn ensure_segment(&mut self) -> io::Result<()> {
        if self.current.is_none() {
            self.current = Some(self.store.create_segment()?);
            self.opened_at = Instant::now();
            for p in self.tables.values_mut() {
                p.seg_table_id = None;
            }
        }
        Ok(())
    }

    fn register_if_needed(&mut self, name: &str, cols: &[(String, DType)]) -> io::Result<u32> {
        if let Some(id) = self.tables[name].seg_table_id {
            return Ok(id);
        }
        let id = self
            .current
            .as_mut()
            .expect("segment open")
            .register_table(name, cols)?;
        self.tables.get_mut(name).unwrap().seg_table_id = Some(id);
        Ok(id)
    }

    /// Move this compactor onto a background thread that drains `sources`
    /// every `poll_interval`, rolls by size/age, and enforces the budget.
    /// Each source is `(table_name, read_handle)`; the handle must be a
    /// shared/file-backed [`MemTable`] the application is writing elsewhere.
    /// Dropping (or [`stop`](CompactorHandle::stop)ping) the returned handle
    /// does a final drain + flush so no sealed chunk is left behind.
    pub fn spawn(mut self, sources: Vec<(String, MemTable)>) -> CompactorHandle {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = stop.clone();
        let poll = self.config.poll_interval;
        let thread = std::thread::Builder::new()
            .name("memc-compactor".into())
            .spawn(move || {
                while !stop_thread.load(Ordering::Relaxed) {
                    for (name, table) in &sources {
                        let view = table.view();
                        let _ = self.drain_view(name, &view);
                    }
                    let _ = self.maybe_roll_on_age();
                    let _ = self.enforce();
                    std::thread::park_timeout(poll);
                }
                for (name, table) in &sources {
                    let view = table.view();
                    let _ = self.drain_view(name, &view);
                }
                let _ = self.flush();
                let _ = self.enforce();
            })
            .expect("spawn memc-compactor thread");
        CompactorHandle {
            stop,
            thread: Some(thread),
        }
    }
}

/// Handle to a background [`Compactor`] thread. Stops and joins on drop.
pub struct CompactorHandle {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl CompactorHandle {
    /// Signal the thread to do a final drain + flush, then join it.
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            t.thread().unpark();
            let _ = t.join();
        }
    }
}

impl Drop for CompactorHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Transpose one chunk's rows into per-column [`ColumnData`].
///
/// Returns `None` if the chunk was empty, never written, or recycled by the
/// ring while we read it (detected by a generation change), so the caller
/// can skip and retry on the next pass without persisting torn data.
fn transpose_chunk(
    view: &MemTableView,
    chunk: usize,
    cols: &[(String, DType)],
) -> Option<(u64, Vec<ColumnData>)> {
    let gen_before = view.chunk_generation(chunk);
    if gen_before == 0 {
        return None;
    }
    let mut builders: Vec<ColumnBuilder> =
        cols.iter().map(|(_, dt)| ColumnBuilder::new(*dt)).collect();

    for row in view.rows(chunk) {
        let mut cur = row.cursor();
        for (ci, (_, dt)) in cols.iter().enumerate() {
            match dt {
                DType::U8 => builders[ci].push(&Value::U8(cur.next_u8())),
                DType::U32 => builders[ci].push(&Value::U32(cur.next_u32())),
                DType::I32 => builders[ci].push(&Value::I32(cur.next_i32())),
                DType::I64 => builders[ci].push(&Value::I64(cur.next_i64())),
                DType::F32 => builders[ci].push(&Value::F32(cur.next_f32())),
                DType::F64 => builders[ci].push(&Value::F64(cur.next_f64())),
                DType::U64 => builders[ci].push(&Value::U64(cur.next_u64())),
                DType::Str => builders[ci].push(&Value::Str(cur.next_str())),
                DType::Bytes => builders[ci].push(&Value::Bytes(cur.next_bytes())),
            }
        }
    }

    if view.chunk_generation(chunk) != gen_before {
        return None; // ring overwrote the chunk mid-transpose
    }
    Some((gen_before, builders.into_iter().map(|b| b.finish()).collect()))
}
