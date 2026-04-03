//! Self-describing row-oriented memory table with chunked ring buffer.
//!
//! Rows are **variable-length** with `u32` length prefixes for fast scanning.
//! Chunks are fixed-size byte blocks; rows are packed sequentially within each chunk.
//!
//! ## Concurrency
//!
//! - **Writers** are serialized by a spinlock (`write_lock` in Header).
//! - **Readers** are lock-free: per-chunk `used` is updated with `Release` ordering
//!   by writers and loaded with `Acquire` by readers, ensuring row data visibility.
//! - `RowWriter` holds the lock for its lifetime; released by `finish()` or `Drop`.
//!
//! # Memory Layout
//!
//! ```text
//! ┌──────────────────────────────────┐ 0
//! │ Header (36 bytes, repr(C))       │
//! │   magic: u32                     │
//! │   version: u32                   │
//! │   num_cols: u32                  │
//! │   num_chunks: u32                │
//! │   write_chunk: AtomicU32         │
//! │   data_offset: u32               │
//! │   chunk_size: u32                │
//! │   write_lock: AtomicU32          │
//! │   refcount: AtomicU32           │
//! ├──────────────────────────────────┤ 36
//! │ ColumnDesc × N (64 bytes each)  │
//! │   name: [u8; 56]  (LP u16)      │
//! │   dtype: u32                     │
//! │   elem_size: u32                 │
//! ├──────────────────────────────────┤ data_offset (64-aligned)
//! │ Chunk 0 (chunk_size bytes)       │
//! │   ChunkHeader (24 bytes)         │
//! │     generation: AtomicU64        │
//! │     used: AtomicU32              │
//! │     row_count: AtomicU32         │
//! │     state: AtomicU32             │
//! │     _reserved: u32               │
//! │   [row_len: u32][col_data...]    │
//! │   ...free space...               │
//! │ Chunk 1 ...                      │
//! └──────────────────────────────────┘
//! ```
//!
//! ## Row Format
//!
//! `[row_len: u32][col_0_data][col_1_data]...[col_N_data]`
//!
//! - Fixed-size columns: raw little-endian bytes
//! - `Str`/`Bytes` columns (inline): `[i32 len ≥ 0][bytes]`
//! - `Str`/`Bytes` columns (dedup ref): `[i32 < 0]` — absolute value is the
//!   offset from chunk start where the original inline `[len][bytes]` lives.
//!   Within a chunk, duplicate strings in the same column are stored as
//!   4-byte references instead of repeated data.
//!
//! # Example
//!
//! ```rust
//! use probing_memtable::{MemTable, Schema, DType, Value};
//!
//! let schema = Schema::new()
//!     .col("ts", DType::I64)
//!     .col("msg", DType::Str);
//!
//! let mut t = MemTable::new(&schema, 4096, 4);
//!
//! // Streaming write — chain put_* calls, no Value allocation
//! t.row_writer().put_i64(1000).put_str("hello").finish();
//!
//! // Or batch write with auto-advance on chunk full
//! t.push_row(&[Value::I64(2000), Value::Str("world")]);
//!
//! // Sequential cursor read — O(1) per column
//! for row in t.rows(0) {
//!     let mut c = row.cursor();
//!     println!("{} {}", c.next_i64(), c.next_str());
//! }
//! ```

use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::mem;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use xxhash_rust::xxh3;

// ── C-style layout structs ──────────────────────────────────────────

const MAGIC: u32 = 0x4D45_4D54; // "MEMT"
const VERSION: u32 = 1;

/// Fixed header at the start of every MemTable buffer.
#[repr(C)]
pub struct Header {
    pub magic: u32,
    pub version: u32,
    pub num_cols: u32,
    pub num_chunks: u32,
    /// Ring buffer: index of the chunk currently being written (atomic).
    pub write_chunk: AtomicU32,
    /// Byte offset where chunk data begins (64-aligned).
    pub data_offset: u32,
    /// Byte budget per chunk (including the per-chunk `used: AtomicU32` header).
    pub chunk_size: u32,
    /// Spinlock for writer serialization: 0 = unlocked, 1 = locked.
    pub write_lock: AtomicU32,
    /// Reference count for shared lifetime management (atomic).
    pub refcount: AtomicU32,
}

/// Per-column descriptor, immediately following the Header.
#[repr(C)]
pub struct ColumnDesc {
    /// Column name, length-prefixed: `[u16 len][utf8 bytes][padding]`.
    pub name: [u8; 56],
    /// `DType` value as `u32`.
    pub dtype: u32,
    /// For fixed-size types: byte size. For `Str`/`Bytes`: 0 (variable-length).
    pub elem_size: u32,
}

/// Per-chunk metadata, at the start of every chunk's byte region.
#[repr(C)]
pub struct ChunkHeader {
    /// Incremented each time the chunk is recycled (ring wrap).
    /// Readers capture this to detect stale reads.
    pub generation: AtomicU64,
    /// Bytes of row data written (excluding this header).
    pub used: AtomicU32,
    /// Number of committed rows in this chunk.
    pub row_count: AtomicU32,
    /// Chunk lifecycle state (see `ChunkState`).
    pub state: AtomicU32,
    pub _reserved: u32,
}

/// Chunk lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum ChunkState {
    Empty = 0,
    Writing = 1,
    Sealed = 2,
}

impl ChunkState {
    fn from_u32(v: u32) -> Self {
        match v {
            0 => Self::Empty,
            1 => Self::Writing,
            2 => Self::Sealed,
            _ => Self::Empty,
        }
    }
}

const CHUNK_HEADER_SIZE: usize = mem::size_of::<ChunkHeader>();

const _: () = {
    assert!(mem::size_of::<Header>() == 36);
    assert!(mem::size_of::<ColumnDesc>() == 64);
    assert!(mem::size_of::<ChunkHeader>() == 24);
};

impl ColumnDesc {
    pub fn name_str(&self) -> &str {
        let len = u16::from_le_bytes([self.name[0], self.name[1]]) as usize;
        if len == 0 {
            return "";
        }
        std::str::from_utf8(&self.name[2..2 + len]).unwrap_or("")
    }

    pub fn set_name(&mut self, s: &str) {
        self.name = [0u8; 56];
        let b = s.as_bytes();
        let n = b.len().min(54);
        self.name[0..2].copy_from_slice(&(n as u16).to_le_bytes());
        self.name[2..2 + n].copy_from_slice(&b[..n]);
    }
}

// ── struct accessors ────────────────────────────────────────────────

fn header(buf: &[u8]) -> &Header {
    debug_assert!(buf.len() >= mem::size_of::<Header>());
    unsafe { &*(buf.as_ptr() as *const Header) }
}

fn header_mut(buf: &mut [u8]) -> &mut Header {
    debug_assert!(buf.len() >= mem::size_of::<Header>());
    unsafe { &mut *(buf.as_mut_ptr() as *mut Header) }
}

fn col_desc(buf: &[u8], col: usize) -> &ColumnDesc {
    let off = mem::size_of::<Header>() + col * mem::size_of::<ColumnDesc>();
    debug_assert!(buf.len() >= off + mem::size_of::<ColumnDesc>());
    unsafe { &*(buf[off..].as_ptr() as *const ColumnDesc) }
}

fn col_desc_mut(buf: &mut [u8], col: usize) -> &mut ColumnDesc {
    let off = mem::size_of::<Header>() + col * mem::size_of::<ColumnDesc>();
    debug_assert!(buf.len() >= off + mem::size_of::<ColumnDesc>());
    unsafe { &mut *(buf[off..].as_mut_ptr() as *mut ColumnDesc) }
}

// ── chunk header accessor ───────────────────────────────────────────

fn chunk_header(buf: &[u8], cs: usize) -> &ChunkHeader {
    debug_assert!(cs % 8 == 0 && buf.len() >= cs + CHUNK_HEADER_SIZE);
    unsafe { &*(buf[cs..].as_ptr() as *const ChunkHeader) }
}

fn acquire_write_lock(buf: &[u8]) {
    let lock = &header(buf).write_lock;
    while lock
        .compare_exchange_weak(0, 1, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        std::hint::spin_loop();
    }
}

fn release_write_lock(buf: &[u8]) {
    header(buf).write_lock.store(0, Ordering::Release);
}

// ── refcount helpers ────────────────────────────────────────────────

/// Read current reference count.
pub fn refcount(buf: &[u8]) -> u32 {
    header(buf).refcount.load(Ordering::Acquire)
}

/// Atomically increment the reference count. Returns the new count.
pub fn acquire_ref(buf: &[u8]) -> u32 {
    header(buf).refcount.fetch_add(1, Ordering::Relaxed) + 1
}

/// Atomically decrement the reference count. Returns the new count.
///
/// When the count drops to zero, an `Acquire` fence ensures all prior
/// accesses from other holders are visible before the caller deallocates.
pub fn release_ref(buf: &[u8]) -> u32 {
    let prev = header(buf).refcount.fetch_sub(1, Ordering::Release);
    debug_assert!(prev > 0, "release_ref on zero refcount");
    if prev == 1 {
        std::sync::atomic::fence(Ordering::Acquire);
    }
    prev - 1
}

// ── data types ──────────────────────────────────────────────────────

/// Column data type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum DType {
    U8 = 1,
    I32 = 2,
    I64 = 3,
    F32 = 4,
    F64 = 5,
    U64 = 6,
    U32 = 7,
    /// Variable-length UTF-8 string. Row entry format: `[u32 len][bytes]`.
    Str = 8,
    /// Variable-length binary buffer. Row entry format: `[u32 len][bytes]`.
    Bytes = 9,
}

impl DType {
    pub fn fixed_size(self) -> Option<usize> {
        match self {
            Self::U8 => Some(1),
            Self::I32 | Self::F32 | Self::U32 => Some(4),
            Self::I64 | Self::F64 | Self::U64 => Some(8),
            Self::Str | Self::Bytes => None,
        }
    }

    pub fn is_fixed(self) -> bool {
        self.fixed_size().is_some()
    }

    fn from_u32(v: u32) -> Self {
        match v {
            1 => Self::U8,
            2 => Self::I32,
            3 => Self::I64,
            4 => Self::F32,
            5 => Self::F64,
            6 => Self::U64,
            7 => Self::U32,
            8 => Self::Str,
            9 => Self::Bytes,
            _ => panic!("invalid DType: {v}"),
        }
    }
}

impl fmt::Display for DType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(match self {
            Self::U8 => "u8",
            Self::I32 => "i32",
            Self::I64 => "i64",
            Self::F32 => "f32",
            Self::F64 => "f64",
            Self::U64 => "u64",
            Self::U32 => "u32",
            Self::Str => "str",
            Self::Bytes => "bytes",
        })
    }
}

// ── schema ──────────────────────────────────────────────────────────

pub struct Col {
    pub name: String,
    pub dtype: DType,
    pub elem_size: usize,
}

pub struct Schema {
    pub cols: Vec<Col>,
}

impl Schema {
    pub fn new() -> Self {
        Self { cols: vec![] }
    }

    pub fn col(mut self, name: &str, dtype: DType) -> Self {
        let elem_size = dtype.fixed_size().unwrap_or(0);
        self.cols.push(Col {
            name: name.into(),
            dtype,
            elem_size,
        });
        self
    }
}

impl Default for Schema {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for Schema {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Schema(")?;
        for (i, c) in self.cols.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{}:{}", c.name, c.dtype)?;
        }
        write!(f, ")")
    }
}

// ── value (for writing rows) ────────────────────────────────────────

/// A typed value for building rows.
pub enum Value<'a> {
    U8(u8),
    U32(u32),
    I32(i32),
    I64(i64),
    F32(f32),
    F64(f64),
    U64(u64),
    Str(&'a str),
    Bytes(&'a [u8]),
}

impl Value<'_> {
    fn encoded_size(&self) -> usize {
        match self {
            Value::U8(_) => 1,
            Value::U32(_) | Value::I32(_) | Value::F32(_) => 4,
            Value::I64(_) | Value::F64(_) | Value::U64(_) => 8,
            Value::Str(s) => 4 + s.len(),
            Value::Bytes(b) => 4 + b.len(),
        }
    }

    fn encode(&self, out: &mut [u8]) {
        match self {
            Value::U8(v) => out[0] = *v,
            Value::U32(v) => out[..4].copy_from_slice(&v.to_le_bytes()),
            Value::I32(v) => out[..4].copy_from_slice(&v.to_le_bytes()),
            Value::I64(v) => out[..8].copy_from_slice(&v.to_le_bytes()),
            Value::F32(v) => out[..4].copy_from_slice(&v.to_le_bytes()),
            Value::F64(v) => out[..8].copy_from_slice(&v.to_le_bytes()),
            Value::U64(v) => out[..8].copy_from_slice(&v.to_le_bytes()),
            Value::Str(s) => {
                let b = s.as_bytes();
                out[..4].copy_from_slice(&(b.len() as u32).to_le_bytes());
                out[4..4 + b.len()].copy_from_slice(b);
            }
            Value::Bytes(b) => {
                out[..4].copy_from_slice(&(b.len() as u32).to_le_bytes());
                out[4..4 + b.len()].copy_from_slice(b);
            }
        }
    }
}

// ── DedupState (must appear before RowWriter) ───────────────────────

/// Per-chunk string/bytes dedup map for streaming or batch writers.
/// Cleared when advancing to the next chunk.
pub struct DedupState {
    seen: HashMap<u64, usize>,
}

impl DedupState {
    pub fn new() -> Self {
        Self { seen: HashMap::new() }
    }

    pub fn clear(&mut self) {
        self.seen.clear();
    }

    fn key(col: usize, data: &[u8]) -> u64 {
        xxh3::xxh3_64_with_seed(data, col as u64)
    }

    fn lookup(&self, col: usize, data: &[u8]) -> Option<usize> {
        if data.is_empty() {
            return None;
        }
        self.seen.get(&Self::key(col, data)).copied()
    }

    fn insert(&mut self, col: usize, data: &[u8], chunk_offset: usize) {
        if !data.is_empty() {
            self.seen.insert(Self::key(col, data), chunk_offset);
        }
    }
}

impl Default for DedupState {
    fn default() -> Self {
        Self::new()
    }
}

// ── RowWriter (streaming write, holds lock; optional dedup) ────────

/// Streaming row writer. Holds the write lock until `finish()` or `Drop`.
/// When created from [`DedupWriter::row_writer`], string columns use hash dedup.
pub struct RowWriter<'a> {
    buf: &'a mut [u8],
    dedup: Option<&'a mut DedupState>,
    chunk_start: usize,
    chunk_size: usize,
    row_start: usize,
    pos: usize,
    overflow: bool,
    done: bool,
    col_idx: usize,
}

impl<'a> RowWriter<'a> {
    fn can_write(&self, n: usize) -> bool {
        !self.overflow && self.pos + n <= self.chunk_start + self.chunk_size
    }

    fn write_raw(&mut self, bytes: &[u8]) {
        if self.can_write(bytes.len()) {
            self.buf[self.pos..self.pos + bytes.len()].copy_from_slice(bytes);
            self.pos += bytes.len();
        } else {
            self.overflow = true;
        }
    }

    fn write_lp(&mut self, data: &[u8]) {
        if self.can_write(4 + data.len()) {
            w32(self.buf, self.pos, data.len() as u32);
            self.buf[self.pos + 4..self.pos + 4 + data.len()].copy_from_slice(data);
            self.pos += 4 + data.len();
        } else {
            self.overflow = true;
        }
    }

    fn bump_col(&mut self) {
        if self.dedup.is_some() {
            self.col_idx += 1;
        }
    }

    fn write_str_dedup(&mut self, data: &[u8]) {
        if !self.overflow {
            if let Some(off) = self.dedup.as_ref().unwrap().lookup(self.col_idx, data) {
                self.write_raw(&(-(off as i32)).to_le_bytes());
                return;
            }
        }
        let chunk_off = self.pos - self.chunk_start;
        self.write_lp(data);
        if !self.overflow {
            self.dedup.as_mut().unwrap().insert(self.col_idx, data, chunk_off);
        }
    }

    pub fn put_u8(&mut self, v: u8) -> &mut Self {
        self.write_raw(&[v]);
        self.bump_col();
        self
    }
    pub fn put_u32(&mut self, v: u32) -> &mut Self {
        self.write_raw(&v.to_le_bytes());
        self.bump_col();
        self
    }
    pub fn put_i32(&mut self, v: i32) -> &mut Self {
        self.write_raw(&v.to_le_bytes());
        self.bump_col();
        self
    }
    pub fn put_i64(&mut self, v: i64) -> &mut Self {
        self.write_raw(&v.to_le_bytes());
        self.bump_col();
        self
    }
    pub fn put_f32(&mut self, v: f32) -> &mut Self {
        self.write_raw(&v.to_le_bytes());
        self.bump_col();
        self
    }
    pub fn put_f64(&mut self, v: f64) -> &mut Self {
        self.write_raw(&v.to_le_bytes());
        self.bump_col();
        self
    }
    pub fn put_u64(&mut self, v: u64) -> &mut Self {
        self.write_raw(&v.to_le_bytes());
        self.bump_col();
        self
    }

    pub fn put_str(&mut self, s: &str) -> &mut Self {
        if self.dedup.is_some() {
            self.write_str_dedup(s.as_bytes());
            self.col_idx += 1;
        } else {
            self.write_lp(s.as_bytes());
        }
        self
    }
    pub fn put_bytes(&mut self, b: &[u8]) -> &mut Self {
        if self.dedup.is_some() {
            self.write_str_dedup(b);
            self.col_idx += 1;
        } else {
            self.write_lp(b);
        }
        self
    }

    /// Commit the row and release the write lock.
    pub fn finish(&mut self) -> bool {
        if self.done {
            return false;
        }
        self.done = true;
        let ok = if self.overflow {
            false
        } else {
            let row_data = self.pos - self.row_start - 4;
            w32(self.buf, self.row_start, row_data as u32);
            let new_used = (self.pos - self.chunk_start - CHUNK_HEADER_SIZE) as u32;
            chunk_header(self.buf, self.chunk_start).used.store(new_used, Ordering::Release);
            chunk_header(self.buf, self.chunk_start)
                .row_count
                .fetch_add(1, Ordering::Release);
            true
        };
        release_write_lock(self.buf);
        ok
    }
}

impl Drop for RowWriter<'_> {
    fn drop(&mut self) {
        if !self.done {
            release_write_lock(self.buf);
        }
    }
}

/// Same as [`RowWriter`]. Dedup is active only when the writer comes from [`DedupWriter::row_writer`].
pub type DedupRowWriter<'a> = RowWriter<'a>;

// ── byte-level helpers ──────────────────────────────────────────────

fn r32(buf: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(buf[off..off + 4].try_into().unwrap())
}

fn w32(buf: &mut [u8], off: usize, v: u32) {
    buf[off..off + 4].copy_from_slice(&v.to_le_bytes());
}

fn align64(n: usize) -> usize {
    (n + 63) & !63
}

// ── layout helpers ──────────────────────────────────────────────────

fn compute_data_offset(num_cols: usize) -> usize {
    align64(mem::size_of::<Header>() + num_cols * mem::size_of::<ColumnDesc>())
}

fn chunk_start_off(buf: &[u8], chunk: usize) -> usize {
    let h = header(buf);
    h.data_offset as usize + chunk * h.chunk_size as usize
}

// ── unlocked write operations (caller must hold write_lock) ─────────

fn append_row_unlocked(buf: &mut [u8], values: &[Value]) -> bool {
    debug_assert!(validate_row_schema(buf, values), "value types do not match schema");
    let h = header(buf);
    let wc = h.write_chunk.load(Ordering::Relaxed) as usize;
    let csz = h.chunk_size as usize;
    let cs = h.data_offset as usize + wc * csz;
    let used = chunk_header(buf, cs).used.load(Ordering::Relaxed) as usize;

    let row_data: usize = values.iter().map(|v| v.encoded_size()).sum();
    let total = 4 + row_data;
    if CHUNK_HEADER_SIZE + used + total > csz {
        return false;
    }

    let row_start = cs + CHUNK_HEADER_SIZE + used;
    w32(buf, row_start, row_data as u32);
    let mut off = row_start + 4;
    for v in values {
        v.encode(&mut buf[off..]);
        off += v.encoded_size();
    }
    chunk_header(buf, cs).used.store((used + total) as u32, Ordering::Release);
    chunk_header(buf, cs).row_count.fetch_add(1, Ordering::Release);
    true
}

fn advance_chunk_unlocked(buf: &[u8]) {
    let h = header(buf);
    let wc = h.write_chunk.load(Ordering::Relaxed);
    let csz = h.chunk_size as usize;
    let doff = h.data_offset as usize;

    // Seal current chunk
    let cur_cs = doff + wc as usize * csz;
    chunk_header(buf, cur_cs).state.store(ChunkState::Sealed as u32, Ordering::Release);

    // Prepare new chunk: bump generation, reset counters
    let new_wc = (wc + 1) % h.num_chunks;
    let cs = doff + new_wc as usize * csz;
    chunk_header(buf, cs).generation.fetch_add(1, Ordering::Relaxed);
    chunk_header(buf, cs).used.store(0, Ordering::Relaxed);
    chunk_header(buf, cs).row_count.store(0, Ordering::Relaxed);
    chunk_header(buf, cs).state.store(ChunkState::Writing as u32, Ordering::Relaxed);

    h.write_chunk.store(new_wc, Ordering::Release);
}

// ── string dedup helpers ────────────────────────────────────────────

/// Encoded size of a variable-length field at `off`: 4 if reference (negative), else 4+len.
fn var_field_size(buf: &[u8], off: usize) -> usize {
    let raw = i32::from_le_bytes(buf[off..off + 4].try_into().unwrap());
    if raw < 0 { 4 } else { 4 + raw as usize }
}

/// Resolve a variable-length field to its actual bytes.
/// If `raw` (i32 at `off`) is negative, follow the reference within the chunk.
fn resolve_var<'a>(buf: &'a [u8], off: usize, chunk_start: usize) -> &'a [u8] {
    let raw = i32::from_le_bytes(buf[off..off + 4].try_into().unwrap());
    if raw < 0 {
        let ref_off = chunk_start + (-raw) as usize;
        let len = r32(buf, ref_off) as usize;
        &buf[ref_off + 4..ref_off + 4 + len]
    } else {
        let len = raw as usize;
        &buf[off + 4..off + 4 + len]
    }
}

// ── Row / RowIter ───────────────────────────────────────────────────

/// Read-only handle to a single row within a chunk.
///
/// Carries the chunk's `generation` at creation time. Access methods use
/// `debug_assert!` to detect stale reads after ring-buffer wrap.
pub struct Row<'a> {
    data: &'a [u8],
    buf: &'a [u8],
    chunk_start: usize,
    generation: u64,
}

impl<'a> Row<'a> {
    pub fn generation(&self) -> u64 { self.generation }

    fn assert_generation(&self) {
        debug_assert_eq!(
            chunk_header(self.buf, self.chunk_start).generation.load(Ordering::Acquire),
            self.generation,
            "stale Row: chunk has been recycled"
        );
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.data
    }

    fn col_offset(&self, col: usize) -> usize {
        self.assert_generation();
        let mut off = 0;
        for i in 0..col {
            let dt = DType::from_u32(col_desc(self.buf, i).dtype);
            if let Some(sz) = dt.fixed_size() {
                off += sz;
            } else {
                off += var_field_size(self.data, off);
            }
        }
        off
    }

    pub fn col_u8(&self, col: usize) -> u8 {
        self.data[self.col_offset(col)]
    }
    pub fn col_u32(&self, col: usize) -> u32 {
        let off = self.col_offset(col);
        u32::from_le_bytes(self.data[off..off + 4].try_into().unwrap())
    }
    pub fn col_i32(&self, col: usize) -> i32 {
        let off = self.col_offset(col);
        i32::from_le_bytes(self.data[off..off + 4].try_into().unwrap())
    }
    pub fn col_i64(&self, col: usize) -> i64 {
        let off = self.col_offset(col);
        i64::from_le_bytes(self.data[off..off + 8].try_into().unwrap())
    }
    pub fn col_f32(&self, col: usize) -> f32 {
        let off = self.col_offset(col);
        f32::from_le_bytes(self.data[off..off + 4].try_into().unwrap())
    }
    pub fn col_f64(&self, col: usize) -> f64 {
        let off = self.col_offset(col);
        f64::from_le_bytes(self.data[off..off + 8].try_into().unwrap())
    }
    pub fn col_u64(&self, col: usize) -> u64 {
        let off = self.col_offset(col);
        u64::from_le_bytes(self.data[off..off + 8].try_into().unwrap())
    }
    pub fn col_str(&self, col: usize) -> &str {
        let off = self.col_offset(col);
        let b = resolve_var(self.buf, self.data.as_ptr() as usize - self.buf.as_ptr() as usize + off, self.chunk_start);
        if b.is_empty() { "" } else { std::str::from_utf8(b).unwrap_or("") }
    }
    pub fn col_bytes(&self, col: usize) -> &[u8] {
        let off = self.col_offset(col);
        resolve_var(self.buf, self.data.as_ptr() as usize - self.buf.as_ptr() as usize + off, self.chunk_start)
    }

    pub fn cursor(&self) -> RowCursor<'a> {
        self.assert_generation();
        RowCursor {
            data: self.data,
            pos: 0,
            buf: self.buf,
            chunk_start: self.chunk_start,
        }
    }
}

/// Sequential cursor over columns within a row — O(1) per column.
pub struct RowCursor<'a> {
    data: &'a [u8],
    pos: usize,
    buf: &'a [u8],
    chunk_start: usize,
}

impl<'a> RowCursor<'a> {
    fn read_fixed<const N: usize>(&mut self) -> [u8; N] {
        let v: [u8; N] = self.data[self.pos..self.pos + N].try_into().unwrap();
        self.pos += N;
        v
    }

    fn read_lp(&mut self) -> &'a [u8] {
        let raw = i32::from_le_bytes(self.read_fixed::<4>());
        if raw < 0 {
            let ref_off = self.chunk_start + (-raw) as usize;
            let len = r32(self.buf, ref_off) as usize;
            &self.buf[ref_off + 4..ref_off + 4 + len]
        } else {
            let len = raw as usize;
            let data = &self.data[self.pos..self.pos + len];
            self.pos += len;
            data
        }
    }

    pub fn next_u8(&mut self) -> u8 { self.read_fixed::<1>()[0] }
    pub fn next_u32(&mut self) -> u32 { u32::from_le_bytes(self.read_fixed()) }
    pub fn next_i32(&mut self) -> i32 { i32::from_le_bytes(self.read_fixed()) }
    pub fn next_i64(&mut self) -> i64 { i64::from_le_bytes(self.read_fixed()) }
    pub fn next_f32(&mut self) -> f32 { f32::from_le_bytes(self.read_fixed()) }
    pub fn next_f64(&mut self) -> f64 { f64::from_le_bytes(self.read_fixed()) }
    pub fn next_u64(&mut self) -> u64 { u64::from_le_bytes(self.read_fixed()) }
    pub fn next_str(&mut self) -> &'a str {
        let b = self.read_lp();
        if b.is_empty() {
            ""
        } else {
            std::str::from_utf8(b).unwrap_or("")
        }
    }
    pub fn next_bytes(&mut self) -> &'a [u8] { self.read_lp() }
}

/// Iterator over rows in a chunk.
///
/// Captures the chunk's `generation` at creation time.
/// Call `is_valid()` to check whether the chunk has been recycled
/// (ring-buffer wrap) since this iterator was created.
pub struct RowIter<'a> {
    buf: &'a [u8],
    chunk_start: usize,
    pos: usize,
    end: usize,
    generation: u64,
}

impl<'a> RowIter<'a> {
    /// The chunk generation captured when this iterator was created.
    pub fn generation(&self) -> u64 { self.generation }

    /// Returns `true` if the chunk's generation still matches the snapshot.
    /// A mismatch means the chunk was recycled and data may be stale.
    pub fn is_valid(&self) -> bool {
        chunk_header(self.buf, self.chunk_start)
            .generation.load(Ordering::Acquire) == self.generation
    }
}

impl<'a> Iterator for RowIter<'a> {
    type Item = Row<'a>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.pos >= self.end {
            return None;
        }
        debug_assert!(self.is_valid(), "stale RowIter: chunk has been recycled");
        let row_len = r32(self.buf, self.pos) as usize;
        let data = &self.buf[self.pos + 4..self.pos + 4 + row_len];
        self.pos += 4 + row_len;
        Some(Row {
            data,
            buf: self.buf,
            chunk_start: self.chunk_start,
            generation: self.generation,
        })
    }
}

// ── helpers on raw buffer (no macro / trait indirection) ────────────

fn begin_row_writer<'a>(buf: &'a mut [u8], dedup: Option<&'a mut DedupState>) -> RowWriter<'a> {
    acquire_write_lock(buf);
    let h = header(buf);
    let wc = h.write_chunk.load(Ordering::Relaxed) as usize;
    let csz = h.chunk_size as usize;
    let doff = h.data_offset as usize;
    let cs = doff + wc * csz;
    let used = chunk_header(buf, cs).used.load(Ordering::Relaxed) as usize;
    RowWriter {
        buf,
        dedup,
        chunk_start: cs,
        chunk_size: csz,
        row_start: cs + CHUNK_HEADER_SIZE + used,
        pos: cs + CHUNK_HEADER_SIZE + used + 4,
        overflow: false,
        done: false,
        col_idx: 0,
    }
}

fn mt_append_row(buf: &mut [u8], values: &[Value]) -> bool {
    assert!(validate_row_schema(buf, values), "value types do not match schema");
    acquire_write_lock(buf);
    let result = append_row_unlocked(buf, values);
    release_write_lock(buf);
    result
}

fn mt_push_row(buf: &mut [u8], values: &[Value]) {
    assert!(validate_row_schema(buf, values), "value types do not match schema");
    acquire_write_lock(buf);
    if !append_row_unlocked(buf, values) {
        advance_chunk_unlocked(buf);
        assert!(
            append_row_unlocked(buf, values),
            "row exceeds chunk capacity"
        );
    }
    release_write_lock(buf);
}

fn mt_advance_chunk(buf: &mut [u8]) {
    acquire_write_lock(buf);
    advance_chunk_unlocked(buf);
    release_write_lock(buf);
}

fn mt_num_cols(buf: &[u8]) -> usize {
    header(buf).num_cols as usize
}
fn mt_num_chunks(buf: &[u8]) -> usize {
    header(buf).num_chunks as usize
}
fn mt_write_chunk(buf: &[u8]) -> usize {
    header(buf).write_chunk.load(Ordering::Acquire) as usize
}
fn mt_data_offset(buf: &[u8]) -> usize {
    header(buf).data_offset as usize
}
fn mt_chunk_size(buf: &[u8]) -> usize {
    header(buf).chunk_size as usize
}
fn mt_col_name(buf: &[u8], i: usize) -> &str {
    col_desc(buf, i).name_str()
}
fn mt_col_dtype(buf: &[u8], i: usize) -> DType {
    DType::from_u32(col_desc(buf, i).dtype)
}
fn mt_col_elem_size(buf: &[u8], i: usize) -> usize {
    col_desc(buf, i).elem_size as usize
}
fn mt_chunk_used(buf: &[u8], chunk: usize) -> usize {
    let cs = chunk_start_off(buf, chunk);
    chunk_header(buf, cs).used.load(Ordering::Acquire) as usize
}
fn mt_chunk_generation(buf: &[u8], chunk: usize) -> u64 {
    let cs = chunk_start_off(buf, chunk);
    chunk_header(buf, cs).generation.load(Ordering::Acquire)
}
fn mt_chunk_state(buf: &[u8], chunk: usize) -> ChunkState {
    let cs = chunk_start_off(buf, chunk);
    ChunkState::from_u32(chunk_header(buf, cs).state.load(Ordering::Acquire))
}
fn mt_chunk_row_count(buf: &[u8], chunk: usize) -> usize {
    let cs = chunk_start_off(buf, chunk);
    chunk_header(buf, cs).row_count.load(Ordering::Acquire) as usize
}
fn mt_rows<'a>(buf: &'a [u8], chunk: usize) -> RowIter<'a> {
    let cs = chunk_start_off(buf, chunk);
    let ch = chunk_header(buf, cs);
    let generation = ch.generation.load(Ordering::Acquire);
    let used = ch.used.load(Ordering::Acquire) as usize;
    RowIter {
        buf,
        chunk_start: cs,
        pos: cs + CHUNK_HEADER_SIZE,
        end: cs + CHUNK_HEADER_SIZE + used,
        generation,
    }
}
fn mt_num_rows(buf: &[u8], chunk: usize) -> usize {
    mt_rows(buf, chunk).count()
}
fn mt_schema(buf: &[u8]) -> Schema {
    let mut s = Schema::new();
    for i in 0..mt_num_cols(buf) {
        s.cols.push(Col {
            name: mt_col_name(buf, i).to_string(),
            dtype: mt_col_dtype(buf, i),
            elem_size: mt_col_elem_size(buf, i),
        });
    }
    s
}

// ── validation ──────────────────────────────────────────────────────

/// Structural validation of a MemTable buffer.
///
/// Checks magic, version, layout offsets, column dtypes, and chunk states.
/// All `from_buf` / `new` constructors funnel through this function.
pub fn validate_buf(buf: &[u8]) -> Result<(), &'static str> {
    if buf.len() < mem::size_of::<Header>() {
        return Err("buffer too small for header");
    }
    let h = header(buf);
    if h.magic != MAGIC {
        return Err("invalid magic");
    }
    if h.version != VERSION {
        return Err("unsupported version");
    }
    let nc = h.num_cols as usize;
    if h.num_chunks == 0 {
        return Err("num_chunks must be > 0");
    }
    let csz = h.chunk_size as usize;
    if csz < CHUNK_HEADER_SIZE + 8 {
        return Err("chunk_size too small");
    }
    let expected_off = compute_data_offset(nc);
    if h.data_offset as usize != expected_off {
        return Err("invalid data_offset");
    }
    let required = expected_off + csz * h.num_chunks as usize;
    if buf.len() < required {
        return Err("buffer too small for data");
    }
    for i in 0..nc {
        let dt = col_desc(buf, i).dtype;
        if !(1..=9).contains(&dt) {
            return Err("invalid column dtype");
        }
    }
    for i in 0..h.num_chunks as usize {
        let cs = expected_off + i * csz;
        let state = chunk_header(buf, cs).state.load(Ordering::Relaxed);
        if state > 2 {
            return Err("invalid chunk state");
        }
    }
    Ok(())
}

/// Check that `values` matches the table schema (column count + dtypes).
fn validate_row_schema(buf: &[u8], values: &[Value]) -> bool {
    let nc = header(buf).num_cols as usize;
    if values.len() != nc {
        return false;
    }
    for (i, v) in values.iter().enumerate() {
        let dt = DType::from_u32(col_desc(buf, i).dtype);
        let ok = matches!(
            (v, dt),
            (Value::U8(_), DType::U8)
                | (Value::U32(_), DType::U32)
                | (Value::I32(_), DType::I32)
                | (Value::I64(_), DType::I64)
                | (Value::F32(_), DType::F32)
                | (Value::F64(_), DType::F64)
                | (Value::U64(_), DType::U64)
                | (Value::Str(_), DType::Str)
                | (Value::Bytes(_), DType::Bytes)
        );
        if !ok {
            return false;
        }
    }
    true
}

// ── init ────────────────────────────────────────────────────────────

fn init_buf(buf: &mut [u8], schema: &Schema, chunk_size: u32, num_chunks: u32) {
    let nc = schema.cols.len();
    let data_off = compute_data_offset(nc);
    let required = data_off + chunk_size as usize * num_chunks as usize;
    assert!(
        buf.len() >= required,
        "buffer too small: need {required} bytes, got {}",
        buf.len()
    );
    assert!(
        chunk_size as usize >= CHUNK_HEADER_SIZE + 8,
        "chunk_size must be at least {} bytes",
        CHUNK_HEADER_SIZE + 8
    );

    let h = header_mut(buf);
    h.magic = MAGIC;
    h.version = VERSION;
    h.num_cols = nc as u32;
    h.num_chunks = num_chunks;
    h.write_chunk.store(0, Ordering::Relaxed);
    h.data_offset = data_off as u32;
    h.chunk_size = chunk_size;
    h.write_lock.store(0, Ordering::Relaxed);
    h.refcount.store(1, Ordering::Relaxed);

    for (i, col) in schema.cols.iter().enumerate() {
        let cd = col_desc_mut(buf, i);
        cd.set_name(&col.name);
        cd.dtype = col.dtype as u32;
        cd.elem_size = col.elem_size as u32;
    }

    // Initialize all chunk headers
    for i in 0..num_chunks as usize {
        let cs = data_off + i * chunk_size as usize;
        let ch = chunk_header(buf, cs);
        ch.generation.store(0, Ordering::Relaxed);
        ch.used.store(0, Ordering::Relaxed);
        ch.row_count.store(0, Ordering::Relaxed);
        ch.state.store(ChunkState::Empty as u32, Ordering::Relaxed);
    }
    // Chunk 0 is the initial write target
    let ch0 = chunk_header(buf, data_off);
    ch0.generation.store(1, Ordering::Relaxed);
    ch0.state.store(ChunkState::Writing as u32, Ordering::Relaxed);
}

// ── MemTable (owned, read + write) ─────────────────────────────────

pub struct MemTable {
    buf: Vec<u8>,
}

impl MemTable {
    pub fn required_size(schema: &Schema, chunk_size: usize, num_chunks: usize) -> usize {
        compute_data_offset(schema.cols.len()) + chunk_size * num_chunks
    }

    pub fn new(schema: &Schema, chunk_size: u32, num_chunks: u32) -> Self {
        let size = Self::required_size(schema, chunk_size as usize, num_chunks as usize);
        let mut buf = vec![0u8; size];
        init_buf(&mut buf, schema, chunk_size, num_chunks);
        Self { buf }
    }

    pub fn from_buf(buf: Vec<u8>) -> Option<Self> {
        validate_buf(&buf).ok()?;
        Some(Self { buf })
    }

    pub fn init_buf(buf: &mut [u8], schema: &Schema, chunk_size: u32, num_chunks: u32) {
        init_buf(buf, schema, chunk_size, num_chunks);
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.buf
    }
    pub fn as_bytes_mut(&mut self) -> &mut [u8] {
        &mut self.buf
    }

    pub fn view(&self) -> MemTableView<'_> {
        MemTableView { buf: &self.buf }
    }

    pub fn num_cols(&self) -> usize {
        mt_num_cols(&self.buf)
    }
    pub fn num_chunks(&self) -> usize {
        mt_num_chunks(&self.buf)
    }
    pub fn write_chunk(&self) -> usize {
        mt_write_chunk(&self.buf)
    }
    pub fn data_offset(&self) -> usize {
        mt_data_offset(&self.buf)
    }
    pub fn chunk_size(&self) -> usize {
        mt_chunk_size(&self.buf)
    }
    pub fn refcount(&self) -> u32 {
        refcount(&self.buf)
    }
    pub fn col_name(&self, i: usize) -> &str {
        mt_col_name(&self.buf, i)
    }
    pub fn col_dtype(&self, i: usize) -> DType {
        mt_col_dtype(&self.buf, i)
    }
    pub fn col_elem_size(&self, i: usize) -> usize {
        mt_col_elem_size(&self.buf, i)
    }
    pub fn chunk_used(&self, chunk: usize) -> usize {
        mt_chunk_used(&self.buf, chunk)
    }
    pub fn chunk_generation(&self, chunk: usize) -> u64 {
        mt_chunk_generation(&self.buf, chunk)
    }
    pub fn chunk_state(&self, chunk: usize) -> ChunkState {
        mt_chunk_state(&self.buf, chunk)
    }
    pub fn chunk_row_count(&self, chunk: usize) -> usize {
        mt_chunk_row_count(&self.buf, chunk)
    }
    pub fn rows(&self, chunk: usize) -> RowIter<'_> {
        mt_rows(&self.buf, chunk)
    }
    pub fn num_rows(&self, chunk: usize) -> usize {
        mt_num_rows(&self.buf, chunk)
    }
    pub fn schema(&self) -> Schema {
        mt_schema(&self.buf)
    }

    pub fn row_writer(&mut self) -> RowWriter<'_> {
        begin_row_writer(self.as_bytes_mut(), None)
    }
    pub fn append_row(&mut self, values: &[Value]) -> bool {
        mt_append_row(self.as_bytes_mut(), values)
    }
    pub fn advance_chunk(&mut self) {
        mt_advance_chunk(self.as_bytes_mut());
    }
    pub fn push_row(&mut self, values: &[Value]) {
        mt_push_row(self.as_bytes_mut(), values);
    }
}

impl fmt::Display for MemTable {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "MemTable({} cols, {} chunks × {} bytes)",
            self.num_cols(),
            self.num_chunks(),
            self.chunk_size()
        )
    }
}

// ── MemTableView (borrowed, read-only) ─────────────────────────────

pub struct MemTableView<'a> {
    buf: &'a [u8],
}

impl<'a> MemTableView<'a> {
    pub fn new(buf: &'a [u8]) -> Option<Self> {
        validate_buf(buf).ok()?;
        Some(Self { buf })
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.buf
    }

    pub fn num_cols(&self) -> usize {
        mt_num_cols(self.buf)
    }
    pub fn num_chunks(&self) -> usize {
        mt_num_chunks(self.buf)
    }
    pub fn write_chunk(&self) -> usize {
        mt_write_chunk(self.buf)
    }
    pub fn data_offset(&self) -> usize {
        mt_data_offset(self.buf)
    }
    pub fn chunk_size(&self) -> usize {
        mt_chunk_size(self.buf)
    }
    pub fn refcount(&self) -> u32 {
        refcount(self.buf)
    }
    pub fn col_name(&self, i: usize) -> &str {
        mt_col_name(self.buf, i)
    }
    pub fn col_dtype(&self, i: usize) -> DType {
        mt_col_dtype(self.buf, i)
    }
    pub fn col_elem_size(&self, i: usize) -> usize {
        mt_col_elem_size(self.buf, i)
    }
    pub fn chunk_used(&self, chunk: usize) -> usize {
        mt_chunk_used(self.buf, chunk)
    }
    pub fn chunk_generation(&self, chunk: usize) -> u64 {
        mt_chunk_generation(self.buf, chunk)
    }
    pub fn chunk_state(&self, chunk: usize) -> ChunkState {
        mt_chunk_state(self.buf, chunk)
    }
    pub fn chunk_row_count(&self, chunk: usize) -> usize {
        mt_chunk_row_count(self.buf, chunk)
    }
    pub fn rows(&self, chunk: usize) -> RowIter<'_> {
        mt_rows(self.buf, chunk)
    }
    pub fn num_rows(&self, chunk: usize) -> usize {
        mt_num_rows(self.buf, chunk)
    }
    pub fn schema(&self) -> Schema {
        mt_schema(self.buf)
    }
}

impl fmt::Display for MemTableView<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "MemTableView({} cols, {} chunks × {} bytes)",
            self.num_cols(),
            self.num_chunks(),
            self.chunk_size()
        )
    }
}

// ── MemTableMut (borrowed, read + write) ────────────────────────────

pub struct MemTableMut<'a> {
    buf: &'a mut [u8],
}

impl<'a> MemTableMut<'a> {
    pub fn new(buf: &'a mut [u8]) -> Option<Self> {
        validate_buf(buf).ok()?;
        Some(Self { buf })
    }

    pub fn init(buf: &'a mut [u8], schema: &Schema, chunk_size: u32, num_chunks: u32) -> Self {
        init_buf(buf, schema, chunk_size, num_chunks);
        Self { buf }
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.buf
    }
    pub fn as_bytes_mut(&mut self) -> &mut [u8] {
        self.buf
    }

    pub fn view(&self) -> MemTableView<'_> {
        MemTableView { buf: self.buf }
    }

    pub fn num_cols(&self) -> usize {
        mt_num_cols(self.buf)
    }
    pub fn num_chunks(&self) -> usize {
        mt_num_chunks(self.buf)
    }
    pub fn write_chunk(&self) -> usize {
        mt_write_chunk(self.buf)
    }
    pub fn data_offset(&self) -> usize {
        mt_data_offset(self.buf)
    }
    pub fn chunk_size(&self) -> usize {
        mt_chunk_size(self.buf)
    }
    pub fn refcount(&self) -> u32 {
        refcount(self.buf)
    }
    pub fn col_name(&self, i: usize) -> &str {
        mt_col_name(self.buf, i)
    }
    pub fn col_dtype(&self, i: usize) -> DType {
        mt_col_dtype(self.buf, i)
    }
    pub fn col_elem_size(&self, i: usize) -> usize {
        mt_col_elem_size(self.buf, i)
    }
    pub fn chunk_used(&self, chunk: usize) -> usize {
        mt_chunk_used(self.buf, chunk)
    }
    pub fn chunk_generation(&self, chunk: usize) -> u64 {
        mt_chunk_generation(self.buf, chunk)
    }
    pub fn chunk_state(&self, chunk: usize) -> ChunkState {
        mt_chunk_state(self.buf, chunk)
    }
    pub fn chunk_row_count(&self, chunk: usize) -> usize {
        mt_chunk_row_count(self.buf, chunk)
    }
    pub fn rows(&self, chunk: usize) -> RowIter<'_> {
        mt_rows(self.buf, chunk)
    }
    pub fn num_rows(&self, chunk: usize) -> usize {
        mt_num_rows(self.buf, chunk)
    }
    pub fn schema(&self) -> Schema {
        mt_schema(self.buf)
    }

    pub fn row_writer(&mut self) -> RowWriter<'_> {
        begin_row_writer(self.as_bytes_mut(), None)
    }
    pub fn append_row(&mut self, values: &[Value]) -> bool {
        mt_append_row(self.as_bytes_mut(), values)
    }
    pub fn advance_chunk(&mut self) {
        mt_advance_chunk(self.as_bytes_mut());
    }
    pub fn push_row(&mut self, values: &[Value]) {
        mt_push_row(self.as_bytes_mut(), values);
    }
}

impl fmt::Display for MemTableMut<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "MemTableMut({} cols, {} chunks × {} bytes)",
            self.num_cols(),
            self.num_chunks(),
            self.chunk_size()
        )
    }
}

// ── DedupWriter (stateful writer with hash dedup) ───────────────────

/// Wraps a mutable buffer and provides hash-based string dedup on writes.
pub struct DedupWriter<'a> {
    buf: &'a mut [u8],
    state: DedupState,
}

impl<'a> DedupWriter<'a> {
    pub fn new(buf: &'a mut [u8]) -> Option<Self> {
        validate_buf(buf).ok()?;
        Some(Self { buf, state: DedupState::new() })
    }

    pub fn init(buf: &'a mut [u8], schema: &Schema, chunk_size: u32, num_chunks: u32) -> Self {
        init_buf(buf, schema, chunk_size, num_chunks);
        Self { buf, state: DedupState::new() }
    }

    pub fn as_bytes(&self) -> &[u8] { self.buf }
    pub fn as_bytes_mut(&mut self) -> &mut [u8] { self.buf }

    pub fn view(&self) -> MemTableView<'_> {
        MemTableView { buf: self.buf }
    }

    pub fn row_writer(&mut self) -> RowWriter<'_> {
        begin_row_writer(self.buf, Some(&mut self.state))
    }

    /// Panics if `values` length or types do not match the schema.
    pub fn push_row(&mut self, values: &[Value]) {
        assert!(validate_row_schema(self.buf, values), "value types do not match schema");
        acquire_write_lock(self.buf);
        if !self.append_row_dedup(values) {
            advance_chunk_unlocked(self.buf);
            self.state.clear();
            assert!(
                self.append_row_dedup(values),
                "row exceeds chunk capacity"
            );
        }
        release_write_lock(self.buf);
    }

    pub fn advance_chunk(&mut self) {
        acquire_write_lock(self.buf);
        advance_chunk_unlocked(self.buf);
        self.state.clear();
        release_write_lock(self.buf);
    }

    fn append_row_dedup(&mut self, values: &[Value]) -> bool {
        debug_assert!(validate_row_schema(self.buf, values), "value types do not match schema");
        let h = header(self.buf);
        let wc = h.write_chunk.load(Ordering::Relaxed) as usize;
        let csz = h.chunk_size as usize;
        let cs = h.data_offset as usize + wc * csz;
        let used = chunk_header(self.buf, cs).used.load(Ordering::Relaxed) as usize;

        let lookups: Vec<Option<usize>> = values.iter().enumerate().map(|(i, v)| match v {
            Value::Str(s) => self.state.lookup(i, s.as_bytes()),
            Value::Bytes(b) => self.state.lookup(i, b),
            _ => None,
        }).collect();

        let row_data: usize = values.iter().zip(&lookups).map(|(v, dup)| {
            if dup.is_some() { 4 } else { v.encoded_size() }
        }).sum();

        let total = 4 + row_data;
        if CHUNK_HEADER_SIZE + used + total > csz {
            return false;
        }

        let row_start = cs + CHUNK_HEADER_SIZE + used;
        w32(self.buf, row_start, row_data as u32);
        let mut off = row_start + 4;
        for (i, v) in values.iter().enumerate() {
            match v {
                Value::Str(s) => {
                    if let Some(ref_off) = lookups[i] {
                        self.buf[off..off + 4].copy_from_slice(&(-(ref_off as i32)).to_le_bytes());
                        off += 4;
                    } else {
                        let chunk_off = off - cs;
                        v.encode(&mut self.buf[off..]);
                        self.state.insert(i, s.as_bytes(), chunk_off);
                        off += v.encoded_size();
                    }
                }
                Value::Bytes(b) => {
                    if let Some(ref_off) = lookups[i] {
                        self.buf[off..off + 4].copy_from_slice(&(-(ref_off as i32)).to_le_bytes());
                        off += 4;
                    } else {
                        let chunk_off = off - cs;
                        v.encode(&mut self.buf[off..]);
                        self.state.insert(i, b, chunk_off);
                        off += v.encoded_size();
                    }
                }
                _ => {
                    v.encode(&mut self.buf[off..]);
                    off += v.encoded_size();
                }
            }
        }
        chunk_header(self.buf, cs).used.store((used + total) as u32, Ordering::Release);
        chunk_header(self.buf, cs).row_count.fetch_add(1, Ordering::Release);
        true
    }

    pub fn num_cols(&self) -> usize {
        mt_num_cols(self.buf)
    }
    pub fn num_chunks(&self) -> usize {
        mt_num_chunks(self.buf)
    }
    pub fn write_chunk(&self) -> usize {
        mt_write_chunk(self.buf)
    }
    pub fn data_offset(&self) -> usize {
        mt_data_offset(self.buf)
    }
    pub fn chunk_size(&self) -> usize {
        mt_chunk_size(self.buf)
    }
    pub fn refcount(&self) -> u32 {
        refcount(self.buf)
    }
    pub fn col_name(&self, i: usize) -> &str {
        mt_col_name(self.buf, i)
    }
    pub fn col_dtype(&self, i: usize) -> DType {
        mt_col_dtype(self.buf, i)
    }
    pub fn col_elem_size(&self, i: usize) -> usize {
        mt_col_elem_size(self.buf, i)
    }
    pub fn chunk_used(&self, chunk: usize) -> usize {
        mt_chunk_used(self.buf, chunk)
    }
    pub fn chunk_generation(&self, chunk: usize) -> u64 {
        mt_chunk_generation(self.buf, chunk)
    }
    pub fn chunk_state(&self, chunk: usize) -> ChunkState {
        mt_chunk_state(self.buf, chunk)
    }
    pub fn chunk_row_count(&self, chunk: usize) -> usize {
        mt_chunk_row_count(self.buf, chunk)
    }
    pub fn rows(&self, chunk: usize) -> RowIter<'_> {
        mt_rows(self.buf, chunk)
    }
    pub fn num_rows(&self, chunk: usize) -> usize {
        mt_num_rows(self.buf, chunk)
    }
    pub fn schema(&self) -> Schema {
        mt_schema(self.buf)
    }
}

impl fmt::Display for DedupWriter<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "DedupWriter({} cols, {} chunks × {} bytes)",
            self.num_cols(),
            self.num_chunks(),
            self.chunk_size()
        )
    }
}

// ── CachedReader (window + pinned string cache) ─────────────────────

/// Cache key: (byte offset, chunk generation).
///
/// Including the generation prevents stale cache hits after ring-buffer wrap.
type CacheKey = (usize, u64);

/// Stateful reader with a generation-aware string cache.
///
/// Caches the most recent `window` resolved strings plus any strings
/// that were resolved via dedup references (pinned). Pinned entries
/// are capped at `max_pinned` to prevent unbounded cache growth.
///
/// Cache keys include the chunk generation so that a recycled chunk
/// at the same offset never produces a stale hit.
pub struct CachedReader<'a> {
    buf: &'a [u8],
    cache: HashMap<CacheKey, &'a [u8]>,
    recent: VecDeque<CacheKey>,
    pinned: HashSet<CacheKey>,
    pinned_order: VecDeque<CacheKey>,
    window: usize,
    max_pinned: usize,
}

impl<'a> CachedReader<'a> {
    pub fn new(buf: &'a [u8], window: usize) -> Self {
        Self::with_limits(buf, window, 4 * window)
    }

    pub fn with_limits(buf: &'a [u8], window: usize, max_pinned: usize) -> Self {
        Self {
            buf,
            cache: HashMap::new(),
            recent: VecDeque::new(),
            pinned: HashSet::new(),
            pinned_order: VecDeque::new(),
            window,
            max_pinned,
        }
    }

    /// Resolve a dedup reference (the target is pinned in cache).
    pub fn resolve_ref(&mut self, data_off: usize, generation: u64) -> &'a [u8] {
        let key = (data_off, generation);
        let cached = self.cache.get(&key).copied();
        if let Some(b) = cached {
            if self.pinned.insert(key) {
                self.pinned_order.push_back(key);
                self.evict();
            }
            return b;
        }
        let len = r32(self.buf, data_off) as usize;
        let b = &self.buf[data_off + 4..data_off + 4 + len];
        self.cache.insert(key, b);
        self.recent.push_back(key);
        self.pinned.insert(key);
        self.pinned_order.push_back(key);
        self.evict();
        b
    }

    /// Cache an inline entry (evictable unless later pinned).
    pub fn cache_inline(&mut self, abs_off: usize, data: &'a [u8], generation: u64) {
        let key = (abs_off, generation);
        if !self.cache.contains_key(&key) {
            self.cache.insert(key, data);
            self.recent.push_back(key);
            self.evict();
        }
    }

    fn evict(&mut self) {
        while self.recent.len() > self.window {
            let old = self.recent.pop_front().unwrap();
            if !self.pinned.contains(&old) {
                self.cache.remove(&old);
            }
        }
        while self.pinned.len() > self.max_pinned {
            if let Some(oldest) = self.pinned_order.pop_front() {
                if self.pinned.remove(&oldest) && !self.recent.contains(&oldest) {
                    self.cache.remove(&oldest);
                }
            } else {
                break;
            }
        }
    }

    pub fn stats(&self) -> (usize, usize) {
        (self.cache.len(), self.pinned.len())
    }

    pub fn cursor(&mut self, row: &Row<'a>) -> CachedCursor<'a, '_> {
        let abs_base = row.data.as_ptr() as usize - row.buf.as_ptr() as usize;
        CachedCursor {
            data: row.data,
            pos: 0,
            abs_base,
            chunk_start: row.chunk_start,
            generation: row.generation,
            cache: self,
        }
    }
}

/// Sequential cursor with generation-aware cached string resolution.
pub struct CachedCursor<'a, 'c> {
    data: &'a [u8],
    pos: usize,
    abs_base: usize,
    chunk_start: usize,
    generation: u64,
    cache: &'c mut CachedReader<'a>,
}

impl<'a> CachedCursor<'a, '_> {
    fn read_fixed<const N: usize>(&mut self) -> [u8; N] {
        let v: [u8; N] = self.data[self.pos..self.pos + N].try_into().unwrap();
        self.pos += N;
        v
    }

    fn read_lp_cached(&mut self) -> &'a [u8] {
        let abs_off = self.abs_base + self.pos;
        let raw = i32::from_le_bytes(self.read_fixed::<4>());
        if raw < 0 {
            let data_off = self.chunk_start + (-raw) as usize;
            self.cache.resolve_ref(data_off, self.generation)
        } else {
            let len = raw as usize;
            let b = &self.data[self.pos..self.pos + len];
            self.pos += len;
            self.cache.cache_inline(abs_off, b, self.generation);
            b
        }
    }

    pub fn next_u8(&mut self) -> u8 { self.read_fixed::<1>()[0] }
    pub fn next_u32(&mut self) -> u32 { u32::from_le_bytes(self.read_fixed()) }
    pub fn next_i32(&mut self) -> i32 { i32::from_le_bytes(self.read_fixed()) }
    pub fn next_i64(&mut self) -> i64 { i64::from_le_bytes(self.read_fixed()) }
    pub fn next_f32(&mut self) -> f32 { f32::from_le_bytes(self.read_fixed()) }
    pub fn next_f64(&mut self) -> f64 { f64::from_le_bytes(self.read_fixed()) }
    pub fn next_u64(&mut self) -> u64 { u64::from_le_bytes(self.read_fixed()) }
    pub fn next_str(&mut self) -> &'a str {
        let b = self.read_lp_cached();
        if b.is_empty() { "" } else { std::str::from_utf8(b).unwrap_or("") }
    }
    pub fn next_bytes(&mut self) -> &'a [u8] { self.read_lp_cached() }
}

// ── tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn struct_sizes() {
        assert_eq!(mem::size_of::<Header>(), 36);
        assert_eq!(mem::size_of::<ColumnDesc>(), 64);
        assert_eq!(mem::size_of::<ChunkHeader>(), 24);
    }

    #[test]
    fn create_and_read_schema() {
        let schema = Schema::new()
            .col("ts", DType::I64)
            .col("value", DType::F64)
            .col("tag", DType::I32);
        let t = MemTable::new(&schema, 4096, 4);
        assert_eq!(t.num_cols(), 3);
        assert_eq!(t.num_chunks(), 4);
        assert_eq!(t.chunk_size(), 4096);
        assert_eq!(t.col_name(0), "ts");
        assert_eq!(t.col_dtype(0), DType::I64);
    }

    #[test]
    fn schema_reconstruct() {
        let schema = Schema::new().col("a", DType::I32).col("b", DType::F64);
        let t = MemTable::new(&schema, 1024, 1);
        let s = t.schema();
        assert_eq!(s.cols.len(), 2);
        assert_eq!(s.cols[0].name, "a");
        assert_eq!(s.cols[1].dtype, DType::F64);
    }

    #[test]
    fn write_and_read_fixed_row() {
        let schema = Schema::new().col("id", DType::I64).col("val", DType::F64);
        let mut t = MemTable::new(&schema, 1024, 1);
        t.push_row(&[Value::I64(42), Value::F64(3.14)]);
        t.push_row(&[Value::I64(100), Value::F64(2.72)]);
        let rows: Vec<_> = t.rows(0).collect();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].col_i64(0), 42);
        assert_eq!(rows[0].col_f64(1), 3.14);
        assert_eq!(rows[1].col_i64(0), 100);
    }

    #[test]
    fn write_and_read_str_column() {
        let schema = Schema::new().col("ts", DType::I64).col("msg", DType::Str);
        let mut t = MemTable::new(&schema, 4096, 1);
        t.push_row(&[Value::I64(1000), Value::Str("hello world")]);
        t.push_row(&[Value::I64(2000), Value::Str("")]);
        t.push_row(&[Value::I64(3000), Value::Str("foo")]);
        let rows: Vec<_> = t.rows(0).collect();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].col_str(1), "hello world");
        assert_eq!(rows[1].col_str(1), "");
        assert_eq!(rows[2].col_str(1), "foo");
    }

    #[test]
    fn bytes_column() {
        let schema = Schema::new().col("data", DType::Bytes);
        let mut t = MemTable::new(&schema, 1024, 1);
        t.push_row(&[Value::Bytes(&[0xDE, 0xAD, 0xBE, 0xEF])]);
        t.push_row(&[Value::Bytes(&[])]);
        let rows: Vec<_> = t.rows(0).collect();
        assert_eq!(rows[0].col_bytes(0), &[0xDE, 0xAD, 0xBE, 0xEF]);
        assert_eq!(rows[1].col_bytes(0), &[]);
    }

    #[test]
    fn u32_column() {
        let schema = Schema::new().col("x", DType::U32);
        let mut t = MemTable::new(&schema, 1024, 1);
        t.push_row(&[Value::U32(0xDEAD_BEEF)]);
        assert_eq!(t.rows(0).next().unwrap().col_u32(0), 0xDEAD_BEEF);
    }

    #[test]
    fn mixed_fixed_and_variable() {
        let schema = Schema::new()
            .col("id", DType::U64)
            .col("name", DType::Str)
            .col("value", DType::F64)
            .col("payload", DType::Bytes);
        let mut t = MemTable::new(&schema, 4096, 1);
        t.push_row(&[
            Value::U64(42),
            Value::Str("test_event"),
            Value::F64(3.14),
            Value::Bytes(&[1, 2, 3]),
        ]);
        let row = t.rows(0).next().unwrap();
        assert_eq!(row.col_u64(0), 42);
        assert_eq!(row.col_str(1), "test_event");
        assert_eq!(row.col_f64(2), 3.14);
        assert_eq!(row.col_bytes(3), &[1, 2, 3]);
    }

    #[test]
    fn variable_length_rows() {
        let schema = Schema::new().col("msg", DType::Str);
        let mut t = MemTable::new(&schema, 4096, 1);
        t.push_row(&[Value::Str("short")]);
        t.push_row(&[Value::Str("a much longer string that takes more space")]);
        t.push_row(&[Value::Str("x")]);
        let rows: Vec<_> = t.rows(0).collect();
        assert_eq!(rows.len(), 3);
        assert_ne!(rows[0].as_bytes().len(), rows[1].as_bytes().len());
    }

    #[test]
    fn chunk_used_tracking() {
        let schema = Schema::new().col("x", DType::I32);
        let mut t = MemTable::new(&schema, 1024, 2);
        assert_eq!(t.chunk_used(0), 0);
        t.push_row(&[Value::I32(1)]);
        assert_eq!(t.chunk_used(0), 8); // 4 (row_len) + 4 (i32)
        t.push_row(&[Value::I32(2)]);
        assert_eq!(t.chunk_used(0), 16);
    }

    #[test]
    fn append_row_returns_false_when_full() {
        let schema = Schema::new().col("x", DType::I64);
        // ChunkHeader=24, each I64 row=12 → 48-24=24 data bytes → 2 rows fit
        let mut t = MemTable::new(&schema, 48, 1);
        assert!(t.append_row(&[Value::I64(1)]));
        assert!(t.append_row(&[Value::I64(2)]));
        assert!(!t.append_row(&[Value::I64(3)]));
        assert_eq!(t.num_rows(0), 2);
    }

    #[test]
    fn ring_buffer_wrap() {
        let schema = Schema::new().col("v", DType::I32);
        // ChunkHeader=24, each I32 row=8 → 80-24=56 data bytes → 7 rows fit
        let mut t = MemTable::new(&schema, 80, 3);
        for i in 0..7 {
            t.push_row(&[Value::I32(i)]);
        }
        assert_eq!(t.write_chunk(), 0);
        assert_eq!(t.num_rows(0), 7);
        t.push_row(&[Value::I32(100)]);
        assert_eq!(t.write_chunk(), 1);
        for i in 0..14 {
            t.push_row(&[Value::I32(200 + i)]);
        }
        assert_eq!(t.write_chunk(), 0);
        assert_eq!(t.rows(0).next().unwrap().col_i32(0), 213);
    }

    #[test]
    fn ring_buffer_with_str() {
        let schema = Schema::new().col("msg", DType::Str);
        let mut t = MemTable::new(&schema, 256, 2);
        for msg in &["alpha", "beta", "gamma", "delta"] {
            t.push_row(&[Value::Str(msg)]);
        }
        assert_eq!(t.rows(0).next().unwrap().col_str(0), "alpha");
    }

    #[test]
    fn view_from_bytes() {
        let schema = Schema::new().col("x", DType::I32).col("s", DType::Str);
        let mut t = MemTable::new(&schema, 4096, 1);
        t.push_row(&[Value::I32(99), Value::Str("view_test")]);
        let view = MemTableView::new(t.as_bytes()).unwrap();
        assert_eq!(view.num_cols(), 2);
        let row = view.rows(0).next().unwrap();
        assert_eq!(row.col_i32(0), 99);
        assert_eq!(row.col_str(1), "view_test");
    }

    #[test]
    fn mut_view_on_external_buf() {
        let schema = Schema::new().col("a", DType::I64).col("b", DType::Str);
        let size = MemTable::required_size(&schema, 4096, 2);
        let mut buf = vec![0u8; size];
        let mut mt = MemTableMut::init(&mut buf, &schema, 4096, 2);
        mt.push_row(&[Value::I64(42), Value::Str("ext_test")]);
        let row = mt.rows(0).next().unwrap();
        assert_eq!(row.col_i64(0), 42);
        assert_eq!(row.col_str(1), "ext_test");
    }

    #[test]
    fn num_rows_count() {
        let schema = Schema::new().col("x", DType::I32);
        let mut t = MemTable::new(&schema, 4096, 1);
        for i in 0..10 {
            t.push_row(&[Value::I32(i)]);
        }
        assert_eq!(t.num_rows(0), 10);
    }

    #[test]
    fn required_size_calculation() {
        let schema = Schema::new().col("a", DType::I64).col("b", DType::F32);
        let size = MemTable::required_size(&schema, 4096, 4);
        assert_eq!(size, 192 + 4096 * 4);
    }

    #[test]
    fn display_format() {
        let schema = Schema::new().col("a", DType::I32);
        let t = MemTable::new(&schema, 1024, 2);
        assert_eq!(format!("{t}"), "MemTable(1 cols, 2 chunks × 1024 bytes)");
    }

    #[test]
    fn schema_debug_format() {
        let schema = Schema::new().col("id", DType::I64).col("name", DType::Str);
        assert_eq!(format!("{schema:?}"), "Schema(id:i64, name:str)");
    }

    #[test]
    fn header_direct_access() {
        let schema = Schema::new().col("x", DType::I32);
        let t = MemTable::new(&schema, 1024, 4);
        let h = header(t.as_bytes());
        assert_eq!(h.magic, MAGIC);
        assert_eq!(h.version, VERSION);
        assert_eq!(h.num_cols, 1);
        assert_eq!(h.num_chunks, 4);
        assert_eq!(h.chunk_size, 1024);
        assert_eq!(h.write_chunk.load(Ordering::Relaxed), 0);
        assert_eq!(h.write_lock.load(Ordering::Relaxed), 0);
        assert_eq!(h.refcount.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn col_desc_direct_access() {
        let schema = Schema::new().col("count", DType::U32).col("tag", DType::Str);
        let t = MemTable::new(&schema, 1024, 1);
        let cd0 = col_desc(t.as_bytes(), 0);
        assert_eq!(cd0.name_str(), "count");
        assert_eq!(cd0.elem_size, 4);
        let cd1 = col_desc(t.as_bytes(), 1);
        assert_eq!(cd1.name_str(), "tag");
        assert_eq!(cd1.elem_size, 0);
    }

    #[test]
    fn invalid_magic_rejected() {
        let buf = vec![0u8; 64];
        assert!(MemTableView::new(&buf).is_none());
        assert!(MemTable::from_buf(buf).is_none());
    }

    #[test]
    fn empty_chunk_iteration() {
        let schema = Schema::new().col("x", DType::I32);
        let t = MemTable::new(&schema, 1024, 2);
        assert_eq!(t.rows(0).count(), 0);
        assert_eq!(t.rows(1).count(), 0);
    }

    #[test]
    fn row_raw_bytes() {
        let schema = Schema::new().col("v", DType::I32);
        let mut t = MemTable::new(&schema, 1024, 1);
        t.push_row(&[Value::I32(0x12345678)]);
        assert_eq!(t.rows(0).next().unwrap().as_bytes(), &0x12345678_i32.to_le_bytes());
    }

    // ── RowWriter / RowCursor tests ─────────────────────────────────

    #[test]
    fn row_writer_basic() {
        let schema = Schema::new().col("id", DType::I64).col("val", DType::F64);
        let mut t = MemTable::new(&schema, 1024, 1);
        assert!(t.row_writer().put_i64(42).put_f64(3.14).finish());
        assert!(t.row_writer().put_i64(100).put_f64(2.72).finish());
        let rows: Vec<_> = t.rows(0).collect();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].col_i64(0), 42);
    }

    #[test]
    fn row_writer_with_str() {
        let schema = Schema::new()
            .col("ts", DType::I64)
            .col("msg", DType::Str)
            .col("tag", DType::U32);
        let mut t = MemTable::new(&schema, 4096, 1);
        t.row_writer().put_i64(1000).put_str("hello").put_u32(7).finish();
        let row = t.rows(0).next().unwrap();
        assert_eq!(row.col_i64(0), 1000);
        assert_eq!(row.col_str(1), "hello");
        assert_eq!(row.col_u32(2), 7);
    }

    #[test]
    fn row_writer_overflow() {
        let schema = Schema::new().col("x", DType::I64);
        // ChunkHeader=24, each I64 row=12 → 40-24=16 → 1 row fits, 2nd overflows
        let mut t = MemTable::new(&schema, 40, 1);
        assert!(t.row_writer().put_i64(1).finish());
        assert!(!t.row_writer().put_i64(2).finish());
        assert_eq!(t.num_rows(0), 1);
    }

    #[test]
    fn row_writer_drop_releases_lock() {
        let schema = Schema::new().col("x", DType::I32);
        let mut t = MemTable::new(&schema, 1024, 1);
        {
            let _w = t.row_writer(); // acquires lock
            // dropped without finish() → lock released by Drop
        }
        // lock should be free; this must not deadlock
        t.push_row(&[Value::I32(42)]);
        assert_eq!(t.rows(0).next().unwrap().col_i32(0), 42);
    }

    #[test]
    fn row_cursor_basic() {
        let schema = Schema::new()
            .col("a", DType::I64)
            .col("b", DType::Str)
            .col("c", DType::F64)
            .col("d", DType::Bytes);
        let mut t = MemTable::new(&schema, 4096, 1);
        t.row_writer()
            .put_i64(42)
            .put_str("test")
            .put_f64(3.14)
            .put_bytes(&[1, 2, 3])
            .finish();
        let row = t.rows(0).next().unwrap();
        let mut c = row.cursor();
        assert_eq!(c.next_i64(), 42);
        assert_eq!(c.next_str(), "test");
        assert_eq!(c.next_f64(), 3.14);
        assert_eq!(c.next_bytes(), &[1, 2, 3]);
    }

    #[test]
    fn cursor_multiple_rows() {
        let schema = Schema::new().col("id", DType::I32).col("name", DType::Str);
        let mut t = MemTable::new(&schema, 4096, 1);
        for i in 0..5 {
            t.row_writer()
                .put_i32(i)
                .put_str(&format!("item_{i}"))
                .finish();
        }
        for (i, row) in t.rows(0).enumerate() {
            let mut c = row.cursor();
            assert_eq!(c.next_i32(), i as i32);
            assert_eq!(c.next_str(), format!("item_{i}"));
        }
    }

    #[test]
    fn writer_and_value_interop() {
        let schema = Schema::new().col("x", DType::I64).col("s", DType::Str);
        let mut t = MemTable::new(&schema, 4096, 1);
        t.row_writer().put_i64(1).put_str("writer").finish();
        t.push_row(&[Value::I64(2), Value::Str("value")]);
        let rows: Vec<_> = t.rows(0).collect();
        let mut c0 = rows[0].cursor();
        assert_eq!(c0.next_i64(), 1);
        assert_eq!(c0.next_str(), "writer");
        let mut c1 = rows[1].cursor();
        assert_eq!(c1.next_i64(), 2);
        assert_eq!(c1.next_str(), "value");
    }

    #[test]
    fn write_lock_field_is_zero_after_operations() {
        let schema = Schema::new().col("x", DType::I32);
        let mut t = MemTable::new(&schema, 1024, 1);
        t.push_row(&[Value::I32(1)]);
        t.row_writer().put_i32(2).finish();
        assert_eq!(
            header(t.as_bytes()).write_lock.load(Ordering::Relaxed),
            0,
            "write_lock must be 0 after all operations complete"
        );
    }

    #[test]
    fn refcount_lifecycle() {
        let schema = Schema::new().col("x", DType::I32);
        let t = MemTable::new(&schema, 1024, 1);
        assert_eq!(t.refcount(), 1);

        assert_eq!(acquire_ref(t.as_bytes()), 2);
        assert_eq!(t.refcount(), 2);

        assert_eq!(release_ref(t.as_bytes()), 1);
        assert_eq!(t.refcount(), 1);

        assert_eq!(release_ref(t.as_bytes()), 0);
    }

    // ── pointer-based expose / register tests ───────────────────────

    #[test]
    fn expose_register_via_pointer() {
        use std::alloc;

        let schema = Schema::new().col("ts", DType::I64).col("msg", DType::Str);
        let size = MemTable::required_size(&schema, 4096, 4);
        let layout = alloc::Layout::from_size_align(size, 64).unwrap();
        let ptr = unsafe { alloc::alloc_zeroed(layout) };
        assert!(!ptr.is_null());

        // ── Component A: expose (init + write) ──
        unsafe {
            let buf = std::slice::from_raw_parts_mut(ptr, size);
            init_buf(buf, &schema, 4096, 4);
            assert_eq!(refcount(buf), 1);

            let mut producer = MemTableMut::new(buf).unwrap();
            for i in 0..10i64 {
                producer
                    .row_writer()
                    .put_i64(i * 100)
                    .put_str(&format!("event_{i}"))
                    .finish();
            }
        }

        // ── Component B: register (receives ptr + size, acquires ref) ──
        unsafe {
            let buf = std::slice::from_raw_parts(ptr, size);
            acquire_ref(buf);
            assert_eq!(refcount(buf), 2);

            let consumer = MemTableView::new(buf).unwrap();
            assert_eq!(consumer.num_cols(), 2);
            assert_eq!(consumer.col_name(0), "ts");

            let mut count = 0i64;
            for row in consumer.rows(0) {
                let mut c = row.cursor();
                assert_eq!(c.next_i64(), count * 100);
                assert_eq!(c.next_str(), format!("event_{count}"));
                count += 1;
            }
            assert_eq!(count, 10);

            assert_eq!(release_ref(buf), 1);
        }

        // ── Component A releases last ref ──
        unsafe {
            let buf = std::slice::from_raw_parts(ptr, size);
            assert_eq!(release_ref(buf), 0);
            alloc::dealloc(ptr, layout);
        }
    }

    #[test]
    fn producer_consumer_threaded() {
        use std::alloc;
        use std::thread;

        let schema = Schema::new().col("seq", DType::I64).col("data", DType::Str);
        let size = MemTable::required_size(&schema, 4096, 4);
        let layout = alloc::Layout::from_size_align(size, 64).unwrap();
        let ptr = unsafe { alloc::alloc_zeroed(layout) };
        assert!(!ptr.is_null());

        // init
        unsafe {
            let buf = std::slice::from_raw_parts_mut(ptr, size);
            init_buf(buf, &schema, 4096, 4);
        }
        // acquire ref for the consumer before spawning
        unsafe { acquire_ref(std::slice::from_raw_parts(ptr, size)) };

        let addr = ptr as usize;
        let producer = thread::spawn(move || {
            let buf = unsafe { std::slice::from_raw_parts_mut(addr as *mut u8, size) };
            let mut mt = MemTableMut::new(buf).unwrap();
            for i in 0..50i64 {
                mt.push_row(&[Value::I64(i), Value::Str("msg")]);
            }
            release_ref(buf);
        });

        producer.join().unwrap();

        // consumer reads after producer is done
        unsafe {
            let buf = std::slice::from_raw_parts(ptr, size);
            let view = MemTableView::new(buf).unwrap();
            let total: usize = (0..view.num_chunks())
                .map(|c| view.num_rows(c))
                .sum();
            assert_eq!(total, 50);

            // verify data
            let mut c = view.rows(0).next().unwrap().cursor();
            assert_eq!(c.next_i64(), 0);
            assert_eq!(c.next_str(), "msg");

            let remaining = release_ref(buf);
            assert_eq!(remaining, 0);
            alloc::dealloc(ptr, layout);
        }
    }

    #[test]
    fn concurrent_multiple_writers() {
        use std::alloc;
        use std::sync::{Arc, Barrier};
        use std::thread;

        let schema = Schema::new().col("tid", DType::I64).col("seq", DType::I64);
        let chunk_size = 8192u32;
        let num_chunks = 8u32;
        let size = MemTable::required_size(&schema, chunk_size as usize, num_chunks as usize);
        let layout = alloc::Layout::from_size_align(size, 64).unwrap();
        let ptr = unsafe { alloc::alloc_zeroed(layout) };
        assert!(!ptr.is_null());

        unsafe {
            let buf = std::slice::from_raw_parts_mut(ptr, size);
            init_buf(buf, &schema, chunk_size, num_chunks);
        }

        let num_writers = 8;
        let rows_per_writer = 50;
        let barrier = Arc::new(Barrier::new(num_writers));
        let addr = ptr as usize;

        let handles: Vec<_> = (0..num_writers)
            .map(|tid| {
                let barrier = barrier.clone();
                thread::spawn(move || {
                    barrier.wait();
                    let buf =
                        unsafe { std::slice::from_raw_parts_mut(addr as *mut u8, size) };
                    let mut mt = MemTableMut::new(buf).unwrap();
                    for seq in 0..rows_per_writer as i64 {
                        mt.push_row(&[Value::I64(tid as i64), Value::I64(seq)]);
                    }
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }

        unsafe {
            let buf = std::slice::from_raw_parts(ptr, size);
            let view = MemTableView::new(buf).unwrap();
            let total: usize = (0..view.num_chunks()).map(|c| view.num_rows(c)).sum();
            assert_eq!(total, num_writers * rows_per_writer);

            // every row should be a valid (tid, seq) pair
            for chunk in 0..view.num_chunks() {
                for row in view.rows(chunk) {
                    let mut c = row.cursor();
                    let tid = c.next_i64();
                    let seq = c.next_i64();
                    assert!((0..num_writers as i64).contains(&tid));
                    assert!((0..rows_per_writer as i64).contains(&seq));
                }
            }

            alloc::dealloc(ptr, layout);
        }
    }

    #[test]
    fn concurrent_writers_and_readers() {
        use std::alloc;
        use std::sync::{Arc, Barrier};
        use std::sync::atomic::{AtomicBool, AtomicUsize};
        use std::thread;

        let schema = Schema::new().col("val", DType::I64);
        let chunk_size = 4096u32;
        let num_chunks = 4u32;
        let size = MemTable::required_size(&schema, chunk_size as usize, num_chunks as usize);
        let layout = alloc::Layout::from_size_align(size, 64).unwrap();
        let ptr = unsafe { alloc::alloc_zeroed(layout) };
        assert!(!ptr.is_null());

        unsafe {
            let buf = std::slice::from_raw_parts_mut(ptr, size);
            init_buf(buf, &schema, chunk_size, num_chunks);
        }

        let num_writers = 4;
        let rows_per_writer = 100;
        let num_readers = 4;
        let addr = ptr as usize;
        let done = Arc::new(AtomicBool::new(false));
        let total_reads = Arc::new(AtomicUsize::new(0));
        let barrier = Arc::new(Barrier::new(num_writers + num_readers));

        // spawn readers — continuously scan all chunks while writers are active
        let reader_handles: Vec<_> = (0..num_readers)
            .map(|_| {
                let done = done.clone();
                let total_reads = total_reads.clone();
                let barrier = barrier.clone();
                thread::spawn(move || {
                    barrier.wait();
                    let mut local_reads = 0usize;
                    while !done.load(Ordering::Acquire) {
                        let buf =
                            unsafe { std::slice::from_raw_parts(addr as *const u8, size) };
                        let view = MemTableView::new(buf).unwrap();
                        for chunk in 0..view.num_chunks() {
                            for row in view.rows(chunk) {
                                let mut c = row.cursor();
                                let v = c.next_i64();
                                assert!(v >= 0, "read corrupt value: {v}");
                                local_reads += 1;
                            }
                        }
                        thread::yield_now();
                    }
                    total_reads.fetch_add(local_reads, Ordering::Relaxed);
                })
            })
            .collect();

        // spawn writers
        let writer_handles: Vec<_> = (0..num_writers)
            .map(|tid| {
                let barrier = barrier.clone();
                thread::spawn(move || {
                    barrier.wait();
                    let buf =
                        unsafe { std::slice::from_raw_parts_mut(addr as *mut u8, size) };
                    let mut mt = MemTableMut::new(buf).unwrap();
                    for seq in 0..rows_per_writer as i64 {
                        mt.push_row(&[Value::I64(tid as i64 * 1000 + seq)]);
                    }
                })
            })
            .collect();

        for h in writer_handles {
            h.join().unwrap();
        }
        done.store(true, Ordering::Release);

        for h in reader_handles {
            h.join().unwrap();
        }

        // readers actually read some rows
        assert!(
            total_reads.load(Ordering::Relaxed) > 0,
            "readers should have observed rows"
        );

        // final consistency: total rows == writers × rows_per_writer
        unsafe {
            let buf = std::slice::from_raw_parts(ptr, size);
            let view = MemTableView::new(buf).unwrap();
            let total: usize = (0..view.num_chunks()).map(|c| view.num_rows(c)).sum();
            assert_eq!(total, num_writers * rows_per_writer);
            alloc::dealloc(ptr, layout);
        }
    }

    #[test]
    fn concurrent_refcount_stress() {
        use std::alloc;
        use std::sync::{Arc, Barrier};
        use std::thread;

        let schema = Schema::new().col("x", DType::I32);
        let size = MemTable::required_size(&schema, 1024, 1);
        let layout = alloc::Layout::from_size_align(size, 64).unwrap();
        let ptr = unsafe { alloc::alloc_zeroed(layout) };
        assert!(!ptr.is_null());

        unsafe {
            let buf = std::slice::from_raw_parts_mut(ptr, size);
            init_buf(buf, &schema, 1024, 1);
        }

        let num_threads = 16;
        let ops_per_thread = 1000;
        let barrier = Arc::new(Barrier::new(num_threads));
        let addr = ptr as usize;

        // each thread: acquire N refs then release N refs
        let handles: Vec<_> = (0..num_threads)
            .map(|_| {
                let barrier = barrier.clone();
                thread::spawn(move || {
                    barrier.wait();
                    let buf =
                        unsafe { std::slice::from_raw_parts(addr as *const u8, size) };
                    for _ in 0..ops_per_thread {
                        acquire_ref(buf);
                    }
                    for _ in 0..ops_per_thread {
                        release_ref(buf);
                    }
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }

        // init set refcount=1, each thread net-zero → should be 1
        unsafe {
            let buf = std::slice::from_raw_parts(ptr, size);
            assert_eq!(refcount(buf), 1);
            release_ref(buf);
            alloc::dealloc(ptr, layout);
        }
    }

    #[test]
    fn concurrent_row_writer_contention() {
        use std::alloc;
        use std::sync::{Arc, Barrier};
        use std::thread;

        let schema = Schema::new()
            .col("tid", DType::I32)
            .col("msg", DType::Str);
        let chunk_size = 16384u32;
        let num_chunks = 4u32;
        let size = MemTable::required_size(&schema, chunk_size as usize, num_chunks as usize);
        let layout = alloc::Layout::from_size_align(size, 64).unwrap();
        let ptr = unsafe { alloc::alloc_zeroed(layout) };
        assert!(!ptr.is_null());

        unsafe {
            let buf = std::slice::from_raw_parts_mut(ptr, size);
            init_buf(buf, &schema, chunk_size, num_chunks);
        }

        let num_writers = 8;
        let rows_per_writer = 60;
        let barrier = Arc::new(Barrier::new(num_writers));
        let addr = ptr as usize;

        let handles: Vec<_> = (0..num_writers)
            .map(|tid| {
                let barrier = barrier.clone();
                thread::spawn(move || {
                    barrier.wait();
                    let buf =
                        unsafe { std::slice::from_raw_parts_mut(addr as *mut u8, size) };
                    let mut mt = MemTableMut::new(buf).unwrap();
                    let tag = format!("t{tid}");
                    for _ in 0..rows_per_writer {
                        mt.row_writer().put_i32(tid as i32).put_str(&tag).finish();
                    }
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }

        unsafe {
            let buf = std::slice::from_raw_parts(ptr, size);
            let view = MemTableView::new(buf).unwrap();
            let total: usize = (0..view.num_chunks()).map(|c| view.num_rows(c)).sum();
            assert_eq!(total, num_writers * rows_per_writer);

            for chunk in 0..view.num_chunks() {
                for row in view.rows(chunk) {
                    let mut c = row.cursor();
                    let tid = c.next_i32();
                    let msg = c.next_str();
                    assert!((0..num_writers as i32).contains(&tid));
                    assert_eq!(msg, format!("t{tid}"));
                }
            }

            alloc::dealloc(ptr, layout);
        }
    }

    // ── string dedup tests (DedupWriter) ──────────────────────────────

    #[test]
    fn dedup_str_saves_space() {
        let schema = Schema::new().col("tag", DType::Str).col("val", DType::I32);
        let size = MemTable::required_size(&schema, 4096, 1);
        let mut buf = vec![0u8; size];
        let mut dw = DedupWriter::init(&mut buf, &schema, 4096, 1);

        dw.push_row(&[Value::Str("hello"), Value::I32(1)]);
        let used_after_first = dw.chunk_used(0);

        dw.push_row(&[Value::Str("hello"), Value::I32(2)]);
        let used_after_second = dw.chunk_used(0);

        dw.push_row(&[Value::Str("world"), Value::I32(3)]);
        let used_after_third = dw.chunk_used(0);

        let second_row_size = used_after_second - used_after_first;
        let third_row_size = used_after_third - used_after_second;
        assert!(second_row_size < third_row_size, "dedup should save: {second_row_size} vs {third_row_size}");
        assert_eq!(second_row_size, 12); // 4+4+4
        assert_eq!(third_row_size, 17);  // 4+(4+5)+4

        let rows: Vec<_> = dw.rows(0).collect();
        assert_eq!(rows[0].col_str(0), "hello");
        assert_eq!(rows[0].col_i32(1), 1);
        assert_eq!(rows[1].col_str(0), "hello");
        assert_eq!(rows[1].col_i32(1), 2);
        assert_eq!(rows[2].col_str(0), "world");
        assert_eq!(rows[2].col_i32(1), 3);
    }

    #[test]
    fn dedup_row_writer_cursor_read() {
        let schema = Schema::new()
            .col("id", DType::I64)
            .col("name", DType::Str)
            .col("status", DType::Str);
        let size = MemTable::required_size(&schema, 4096, 1);
        let mut buf = vec![0u8; size];
        let mut dw = DedupWriter::init(&mut buf, &schema, 4096, 1);

        dw.row_writer().put_i64(1).put_str("alice").put_str("active").finish();
        dw.row_writer().put_i64(2).put_str("bob").put_str("active").finish();
        dw.row_writer().put_i64(3).put_str("alice").put_str("inactive").finish();

        for (i, row) in dw.rows(0).enumerate() {
            let mut c = row.cursor();
            let id = c.next_i64();
            let name = c.next_str();
            let status = c.next_str();
            match i {
                0 => { assert_eq!(id, 1); assert_eq!(name, "alice"); assert_eq!(status, "active"); }
                1 => { assert_eq!(id, 2); assert_eq!(name, "bob"); assert_eq!(status, "active"); }
                2 => { assert_eq!(id, 3); assert_eq!(name, "alice"); assert_eq!(status, "inactive"); }
                _ => unreachable!(),
            }
        }
    }

    #[test]
    fn dedup_bytes_column() {
        let schema = Schema::new().col("payload", DType::Bytes);
        let size = MemTable::required_size(&schema, 4096, 1);
        let mut buf = vec![0u8; size];
        let mut dw = DedupWriter::init(&mut buf, &schema, 4096, 1);

        let data = &[0xDE, 0xAD, 0xBE, 0xEF];
        dw.push_row(&[Value::Bytes(data)]);
        let u1 = dw.chunk_used(0);
        dw.push_row(&[Value::Bytes(data)]);
        let u2 = dw.chunk_used(0);
        dw.push_row(&[Value::Bytes(&[0xFF])]);

        assert_eq!(u2 - u1, 8); // 4 row_len + 4 ref
        let rows: Vec<_> = dw.rows(0).collect();
        assert_eq!(rows[0].col_bytes(0), data);
        assert_eq!(rows[1].col_bytes(0), data);
        assert_eq!(rows[2].col_bytes(0), &[0xFF]);
    }

    #[test]
    fn dedup_empty_str_not_deduped() {
        let schema = Schema::new().col("s", DType::Str);
        let size = MemTable::required_size(&schema, 4096, 1);
        let mut buf = vec![0u8; size];
        let mut dw = DedupWriter::init(&mut buf, &schema, 4096, 1);
        dw.push_row(&[Value::Str("")]);
        dw.push_row(&[Value::Str("")]);
        assert_eq!(dw.chunk_used(0), 16); // both inline: 8+8
        assert_eq!(dw.rows(0).next().unwrap().col_str(0), "");
    }

    #[test]
    fn dedup_across_chunk_boundary_resets() {
        let schema = Schema::new().col("tag", DType::Str);
        let size = MemTable::required_size(&schema, 128, 2);
        let mut buf = vec![0u8; size];
        let mut dw = DedupWriter::init(&mut buf, &schema, 128, 2);

        for _ in 0..5 {
            dw.push_row(&[Value::Str("repeat")]);
        }
        dw.advance_chunk();

        // chunk 1: first inline, second dedup
        dw.push_row(&[Value::Str("repeat")]);
        dw.push_row(&[Value::Str("repeat")]);

        let rows_c1: Vec<_> = dw.rows(1).collect();
        assert_eq!(rows_c1.len(), 2);
        assert_eq!(rows_c1[0].col_str(0), "repeat");
        assert_eq!(rows_c1[1].col_str(0), "repeat");
        assert_eq!(dw.chunk_used(1), 14 + 8); // 14 inline + 8 dedup
    }

    #[test]
    fn dedup_many_duplicates() {
        let schema = Schema::new().col("level", DType::Str).col("msg", DType::Str);
        let size = MemTable::required_size(&schema, 8192, 1);
        let mut buf = vec![0u8; size];
        let mut dw = DedupWriter::init(&mut buf, &schema, 8192, 1);

        let levels = ["INFO", "WARN", "ERROR"];
        for i in 0..30 {
            dw.row_writer()
                .put_str(levels[i % 3])
                .put_str(&format!("message_{i}"))
                .finish();
        }

        for (i, row) in dw.rows(0).enumerate() {
            let mut c = row.cursor();
            assert_eq!(c.next_str(), levels[i % 3]);
            assert_eq!(c.next_str(), format!("message_{i}"));
        }
        assert_eq!(dw.num_rows(0), 30);
    }

    // ── CachedReader tests ──────────────────────────────────────────

    #[test]
    fn cached_reader_basic() {
        let schema = Schema::new().col("id", DType::I64).col("tag", DType::Str);
        let size = MemTable::required_size(&schema, 4096, 1);
        let mut buf = vec![0u8; size];
        {
            let mut dw = DedupWriter::init(&mut buf, &schema, 4096, 1);
            for i in 0..10i64 {
                dw.row_writer().put_i64(i).put_str("same_tag").finish();
            }
        }

        let view = MemTableView::new(&buf).unwrap();
        let mut cache = CachedReader::new(view.as_bytes(), 64);

        for (i, row) in view.rows(0).enumerate() {
            let mut c = cache.cursor(&row);
            assert_eq!(c.next_i64(), i as i64);
            assert_eq!(c.next_str(), "same_tag");
        }

        let (cached, pinned) = cache.stats();
        assert!(pinned > 0, "referenced strings should be pinned");
        assert!(cached > 0);
    }

    #[test]
    fn cached_reader_window_eviction() {
        let schema = Schema::new().col("name", DType::Str);
        let size = MemTable::required_size(&schema, 16384, 1);
        let mut buf = vec![0u8; size];
        {
            let mut dw = DedupWriter::init(&mut buf, &schema, 16384, 1);
            // Write 100 unique strings, no dedup → all inline
            for i in 0..100 {
                dw.push_row(&[Value::Str(&format!("unique_{i}"))]);
            }
        }

        // window=8: should evict old entries
        let view = MemTableView::new(&buf).unwrap();
        let mut cache = CachedReader::new(view.as_bytes(), 8);

        for (i, row) in view.rows(0).enumerate() {
            let mut c = cache.cursor(&row);
            assert_eq!(c.next_str(), format!("unique_{i}"));
        }

        let (cached, pinned) = cache.stats();
        assert!(cached <= 8, "cache should respect window: got {cached}");
        assert_eq!(pinned, 0, "no dedup refs → no pinned entries");
    }

    #[test]
    fn cached_reader_pinned_not_evicted() {
        let schema = Schema::new().col("level", DType::Str).col("seq", DType::I32);
        let size = MemTable::required_size(&schema, 8192, 1);
        let mut buf = vec![0u8; size];
        {
            let mut dw = DedupWriter::init(&mut buf, &schema, 8192, 1);
            // "INFO" repeated → dedup refs after first
            for i in 0..20 {
                dw.push_row(&[Value::Str("INFO"), Value::I32(i)]);
            }
        }

        // window=4 but pinned entries survive eviction
        let view = MemTableView::new(&buf).unwrap();
        let mut cache = CachedReader::new(view.as_bytes(), 4);

        for (i, row) in view.rows(0).enumerate() {
            let mut c = cache.cursor(&row);
            assert_eq!(c.next_str(), "INFO");
            assert_eq!(c.next_i32(), i as i32);
        }

        let (_cached, pinned) = cache.stats();
        assert!(pinned > 0, "dedup target should be pinned");
    }

    #[test]
    fn cached_reader_pinned_limit() {
        let schema = Schema::new().col("tag", DType::Str).col("seq", DType::I32);
        let size = MemTable::required_size(&schema, 65536, 1);
        let mut buf = vec![0u8; size];
        {
            let mut dw = DedupWriter::init(&mut buf, &schema, 65536, 1);
            // 100 unique tags, each repeated twice → 100 pinned targets
            for i in 0..100 {
                let tag = format!("tag_{i}");
                dw.push_row(&[Value::Str(&tag), Value::I32(i as i32)]);
                dw.push_row(&[Value::Str(&tag), Value::I32(i as i32 + 1000)]);
            }
        }

        // max_pinned=10 → pinned entries should be capped
        let view = MemTableView::new(&buf).unwrap();
        let mut cache = CachedReader::with_limits(view.as_bytes(), 8, 10);

        for row in view.rows(0) {
            let mut c = cache.cursor(&row);
            let tag = c.next_str();
            let _seq = c.next_i32();
            assert!(tag.starts_with("tag_"));
        }

        let (_cached, pinned) = cache.stats();
        assert!(
            pinned <= 10,
            "pinned should be capped at max_pinned=10, got {pinned}"
        );
    }

    #[test]
    fn init_buf_rejects_small_buffer() {
        let schema = Schema::new().col("x", DType::I32);
        let result = std::panic::catch_unwind(|| {
            let mut buf = vec![0u8; 32]; // way too small
            init_buf(&mut buf, &schema, 1024, 1);
        });
        assert!(result.is_err(), "init_buf should panic on undersized buffer");
    }

    #[test]
    fn version_field_set() {
        let schema = Schema::new().col("x", DType::I32);
        let t = MemTable::new(&schema, 1024, 1);
        assert_eq!(header(t.as_bytes()).version, VERSION);
    }

    // ── chunk header tests ──────────────────────────────────────────

    #[test]
    fn chunk_header_init_state() {
        let schema = Schema::new().col("x", DType::I32);
        let t = MemTable::new(&schema, 1024, 4);
        // Chunk 0 = Writing, generation 1
        assert_eq!(t.chunk_state(0), ChunkState::Writing);
        assert_eq!(t.chunk_generation(0), 1);
        assert_eq!(t.chunk_row_count(0), 0);
        // Other chunks = Empty, generation 0
        assert_eq!(t.chunk_state(1), ChunkState::Empty);
        assert_eq!(t.chunk_generation(1), 0);
    }

    #[test]
    fn chunk_state_transitions() {
        let schema = Schema::new().col("v", DType::I32);
        let mut t = MemTable::new(&schema, 1024, 3);

        // Write some rows to chunk 0
        t.push_row(&[Value::I32(1)]);
        t.push_row(&[Value::I32(2)]);
        assert_eq!(t.chunk_state(0), ChunkState::Writing);
        assert_eq!(t.chunk_row_count(0), 2);

        // Advance: chunk 0 → Sealed, chunk 1 → Writing
        t.advance_chunk();
        assert_eq!(t.chunk_state(0), ChunkState::Sealed);
        assert_eq!(t.chunk_state(1), ChunkState::Writing);
        assert_eq!(t.chunk_generation(1), 1);
    }

    #[test]
    fn chunk_generation_increments_on_wrap() {
        let schema = Schema::new().col("v", DType::I32);
        // 80 bytes per chunk → 7 I32 rows per chunk
        let mut t = MemTable::new(&schema, 80, 2);

        assert_eq!(t.chunk_generation(0), 1);
        assert_eq!(t.chunk_generation(1), 0);

        // Fill chunk 0 (7 rows), then push_row triggers advance to chunk 1
        for i in 0..8 {
            t.push_row(&[Value::I32(i)]);
        }
        assert_eq!(t.write_chunk(), 1);
        assert_eq!(t.chunk_generation(1), 1);

        // Fill chunk 1 (7 rows), then advance back to chunk 0
        for i in 0..8 {
            t.push_row(&[Value::I32(100 + i)]);
        }
        assert_eq!(t.write_chunk(), 0);
        // Chunk 0 was recycled: generation bumped from 1 to 2
        assert_eq!(t.chunk_generation(0), 2);
        assert_eq!(t.chunk_state(0), ChunkState::Writing);
    }

    #[test]
    fn row_iter_is_valid_detects_wrap() {
        let schema = Schema::new().col("v", DType::I32);
        let size = MemTable::required_size(&schema, 80, 2);
        let mut buf = vec![0u8; size];
        let mut mt = MemTableMut::init(&mut buf, &schema, 80, 2);

        for i in 0..3 {
            mt.push_row(&[Value::I32(i)]);
        }

        // Capture generation of chunk 0
        let gen0 = mt.chunk_generation(0);

        // Advance twice: chunk 0 gets recycled
        mt.advance_chunk();
        mt.advance_chunk();

        // Generation changed → stale
        assert_ne!(mt.chunk_generation(0), gen0);
        assert_eq!(mt.chunk_generation(0), gen0 + 1);
    }

    #[test]
    fn chunk_row_count_matches_iteration() {
        let schema = Schema::new().col("id", DType::I64).col("msg", DType::Str);
        let mut t = MemTable::new(&schema, 4096, 2);
        for i in 0..20i64 {
            t.push_row(&[Value::I64(i), Value::Str("hello")]);
        }
        assert_eq!(t.chunk_row_count(0), t.num_rows(0));
    }

    // ── stress tests ────────────────────────────────────────────────

    #[test]
    fn stress_dedup_writer_large_volume() {
        let schema = Schema::new()
            .col("ts", DType::I64)
            .col("level", DType::Str)
            .col("component", DType::Str)
            .col("msg", DType::Str);
        let size = MemTable::required_size(&schema, 65536, 8);
        let mut buf = vec![0u8; size];
        let mut dw = DedupWriter::init(&mut buf, &schema, 65536, 8);

        let levels = ["TRACE", "DEBUG", "INFO", "WARN", "ERROR"];
        let components = ["http", "db", "cache", "auth", "scheduler", "worker"];
        let n = 2000;

        for i in 0..n as i64 {
            dw.push_row(&[
                Value::I64(i),
                Value::Str(levels[i as usize % levels.len()]),
                Value::Str(components[i as usize % components.len()]),
                Value::Str(&format!("event_{}", i % 200)),
            ]);
        }

        // verify every row is readable and correct
        let mut total = 0;
        for chunk in 0..dw.num_chunks() {
            for row in dw.rows(chunk) {
                let mut c = row.cursor();
                let ts = c.next_i64();
                let level = c.next_str();
                let comp = c.next_str();
                let msg = c.next_str();
                assert!(levels.contains(&level), "bad level: {level}");
                assert!(components.contains(&comp), "bad component: {comp}");
                assert!(msg.starts_with("event_"), "bad msg: {msg}");
                assert!(ts >= 0);
                total += 1;
            }
        }
        assert!(total > 0, "should have rows");
    }

    #[test]
    fn stress_ring_buffer_wrap_with_dedup() {
        let schema = Schema::new().col("tag", DType::Str).col("seq", DType::I64);
        // tiny chunks → frequent wraps
        let size = MemTable::required_size(&schema, 256, 4);
        let mut buf = vec![0u8; size];
        let mut dw = DedupWriter::init(&mut buf, &schema, 256, 4);

        let tags = ["alpha", "beta", "gamma"];
        for i in 0..500i64 {
            dw.push_row(&[Value::Str(tags[i as usize % 3]), Value::I64(i)]);
        }

        // at least some chunks should have data; ring wraps many times
        let mut any_rows = false;
        for chunk in 0..dw.num_chunks() {
            for row in dw.rows(chunk) {
                let mut c = row.cursor();
                let tag = c.next_str();
                let _seq = c.next_i64();
                assert!(tags.contains(&tag));
                any_rows = true;
            }
        }
        assert!(any_rows);
    }

    #[test]
    fn stress_concurrent_dedup_writers() {
        use std::alloc;
        use std::sync::{Arc, Barrier};
        use std::thread;

        let schema = Schema::new()
            .col("tid", DType::I32)
            .col("tag", DType::Str)
            .col("seq", DType::I64);
        let size = MemTable::required_size(&schema, 32768, 8);
        let layout = alloc::Layout::from_size_align(size, 64).unwrap();
        let ptr = unsafe { alloc::alloc_zeroed(layout) };
        assert!(!ptr.is_null());

        unsafe {
            let buf = std::slice::from_raw_parts_mut(ptr, size);
            init_buf(buf, &schema, 32768, 8);
        }

        let num_threads = 8;
        let rows_per_thread = 200;
        let barrier = Arc::new(Barrier::new(num_threads));
        let addr = ptr as usize;

        let handles: Vec<_> = (0..num_threads)
            .map(|tid| {
                let barrier = barrier.clone();
                thread::spawn(move || {
                    barrier.wait();
                    let buf = unsafe { std::slice::from_raw_parts_mut(addr as *mut u8, size) };
                    // each thread uses its own DedupState, but writes to shared buffer
                    // write lock serializes actual writes
                    let mut state = DedupState::new();
                    let tags = ["A", "B", "C", "D"];
                    for seq in 0..rows_per_thread as i64 {
                        acquire_write_lock(buf);
                        let h = header(buf);
                        let wc = h.write_chunk.load(Ordering::Relaxed) as usize;
                        let csz = h.chunk_size as usize;
                        let cs = h.data_offset as usize + wc * csz;
                        let used = chunk_header(buf, cs).used.load(Ordering::Relaxed) as usize;

                        let tag = tags[seq as usize % tags.len()];
                        let values = [Value::I32(tid as i32), Value::Str(tag), Value::I64(seq)];
                        let row_data: usize = values.iter().enumerate().map(|(i, v)| {
                            match v {
                                Value::Str(s) if state.lookup(i, s.as_bytes()).is_some() => 4,
                                _ => v.encoded_size(),
                            }
                        }).sum();
                        let total = 4 + row_data;

                        if CHUNK_HEADER_SIZE + used + total > csz {
                            advance_chunk_unlocked(buf);
                            state.clear();
                            let wc2 = header(buf).write_chunk.load(Ordering::Relaxed) as usize;
                            let cs2 = header(buf).data_offset as usize + wc2 * csz;
                            let used2 = chunk_header(buf, cs2).used.load(Ordering::Relaxed) as usize;
                            let row_start = cs2 + CHUNK_HEADER_SIZE + used2;
                            w32(buf, row_start, row_data as u32);
                            let mut off = row_start + 4;
                            for (i, v) in values.iter().enumerate() {
                                match v {
                                    Value::Str(s) => {
                                        let chunk_off = off - cs2;
                                        v.encode(&mut buf[off..]);
                                        state.insert(i, s.as_bytes(), chunk_off);
                                        off += v.encoded_size();
                                    }
                                    _ => { v.encode(&mut buf[off..]); off += v.encoded_size(); }
                                }
                            }
                            chunk_header(buf, cs2).used.store((used2 + total) as u32, Ordering::Release);
                            chunk_header(buf, cs2).row_count.fetch_add(1, Ordering::Release);
                        } else {
                            let row_start = cs + CHUNK_HEADER_SIZE + used;
                            w32(buf, row_start, row_data as u32);
                            let mut off = row_start + 4;
                            for (i, v) in values.iter().enumerate() {
                                match v {
                                    Value::Str(s) => {
                                        if let Some(ref_off) = state.lookup(i, s.as_bytes()) {
                                            buf[off..off+4].copy_from_slice(&(-(ref_off as i32)).to_le_bytes());
                                            off += 4;
                                        } else {
                                            let chunk_off = off - cs;
                                            v.encode(&mut buf[off..]);
                                            state.insert(i, s.as_bytes(), chunk_off);
                                            off += v.encoded_size();
                                        }
                                    }
                                    _ => { v.encode(&mut buf[off..]); off += v.encoded_size(); }
                                }
                            }
                            chunk_header(buf, cs).used.store((used + total) as u32, Ordering::Release);
                            chunk_header(buf, cs).row_count.fetch_add(1, Ordering::Release);
                        }
                        release_write_lock(buf);
                    }
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }

        unsafe {
            let buf = std::slice::from_raw_parts(ptr, size);
            let view = MemTableView::new(buf).unwrap();
            let total: usize = (0..view.num_chunks()).map(|c| view.num_rows(c)).sum();
            assert_eq!(total, num_threads * rows_per_thread);

            let tags = ["A", "B", "C", "D"];
            for chunk in 0..view.num_chunks() {
                for row in view.rows(chunk) {
                    let mut c = row.cursor();
                    let tid = c.next_i32();
                    let tag = c.next_str();
                    let _seq = c.next_i64();
                    assert!((0..num_threads as i32).contains(&tid));
                    assert!(tags.contains(&tag), "corrupt tag: {tag}");
                }
            }

            alloc::dealloc(ptr as *mut u8, layout);
        }
    }

    #[test]
    fn stress_concurrent_dedup_write_cached_read() {
        use std::alloc;
        use std::sync::atomic::AtomicBool;
        use std::sync::{Arc, Barrier};
        use std::thread;

        let schema = Schema::new()
            .col("key", DType::Str)
            .col("val", DType::I64);
        let size = MemTable::required_size(&schema, 16384, 4);
        let layout = alloc::Layout::from_size_align(size, 64).unwrap();
        let ptr = unsafe { alloc::alloc_zeroed(layout) };
        assert!(!ptr.is_null());

        unsafe {
            let buf = std::slice::from_raw_parts_mut(ptr, size);
            init_buf(buf, &schema, 16384, 4);
        }

        let addr = ptr as usize;
        let num_writers = 4;
        let rows_per_writer = 300;
        let num_readers = 4;
        let done = Arc::new(AtomicBool::new(false));
        let barrier = Arc::new(Barrier::new(num_writers + num_readers));

        let reader_handles: Vec<_> = (0..num_readers)
            .map(|_| {
                let done = done.clone();
                let barrier = barrier.clone();
                thread::spawn(move || {
                    barrier.wait();
                    let mut reads = 0usize;
                    let keys = ["k_a", "k_b", "k_c", "k_d", "k_e"];
                    while !done.load(Ordering::Acquire) {
                        let buf = unsafe { std::slice::from_raw_parts(addr as *const u8, size) };
                        let view = MemTableView::new(buf).unwrap();
                        let mut cache = CachedReader::new(buf, 32);
                        for chunk in 0..view.num_chunks() {
                            for row in view.rows(chunk) {
                                let mut c = cache.cursor(&row);
                                let key = c.next_str();
                                let val = c.next_i64();
                                assert!(
                                    keys.contains(&key) || key.is_empty(),
                                    "corrupt key: {key}"
                                );
                                assert!(val >= 0, "corrupt val: {val}");
                                reads += 1;
                            }
                        }
                        thread::yield_now();
                    }
                    reads
                })
            })
            .collect();

        let writer_handles: Vec<_> = (0..num_writers)
            .map(|tid| {
                let barrier = barrier.clone();
                thread::spawn(move || {
                    barrier.wait();
                    let buf = unsafe { std::slice::from_raw_parts_mut(addr as *mut u8, size) };
                    let mut mt = MemTableMut::new(buf).unwrap();
                    let keys = ["k_a", "k_b", "k_c", "k_d", "k_e"];
                    for seq in 0..rows_per_writer as i64 {
                        mt.push_row(&[
                            Value::Str(keys[seq as usize % keys.len()]),
                            Value::I64(tid as i64 * 10000 + seq),
                        ]);
                    }
                })
            })
            .collect();

        for h in writer_handles {
            h.join().unwrap();
        }
        done.store(true, Ordering::Release);

        let mut total_reads = 0usize;
        for h in reader_handles {
            total_reads += h.join().unwrap();
        }
        assert!(total_reads > 0, "readers should have read some rows");

        unsafe {
            let buf = std::slice::from_raw_parts(ptr, size);
            let view = MemTableView::new(buf).unwrap();
            let total: usize = (0..view.num_chunks()).map(|c| view.num_rows(c)).sum();
            assert_eq!(total, num_writers * rows_per_writer);
            alloc::dealloc(ptr as *mut u8, layout);
        }
    }

    #[test]
    fn stress_cached_reader_high_cardinality_with_dedup() {
        let schema = Schema::new()
            .col("host", DType::Str)
            .col("path", DType::Str)
            .col("status", DType::I32);
        let size = MemTable::required_size(&schema, 65536, 2);
        let mut buf = vec![0u8; size];
        let hosts: Vec<String> = (0..10).map(|i| format!("host-{i}.example.com")).collect();
        let paths: Vec<String> = (0..50).map(|i| format!("/api/v1/resource/{i}")).collect();

        {
            let mut dw = DedupWriter::init(&mut buf, &schema, 65536, 2);
            for i in 0..1000 {
                dw.push_row(&[
                    Value::Str(&hosts[i % hosts.len()]),
                    Value::Str(&paths[i % paths.len()]),
                    Value::I32((200 + (i % 5) * 100) as i32),
                ]);
            }
        }

        // small window → heavy eviction pressure; pinned hosts/paths survive
        let view = MemTableView::new(&buf).unwrap();
        let mut cache = CachedReader::new(view.as_bytes(), 16);
        let mut count = 0;

        for chunk in 0..view.num_chunks() {
            for row in view.rows(chunk) {
                let mut c = cache.cursor(&row);
                let host = c.next_str();
                let path = c.next_str();
                let status = c.next_i32();
                assert!(host.starts_with("host-"), "bad host: {host}");
                assert!(path.starts_with("/api/"), "bad path: {path}");
                assert!([200, 300, 400, 500, 600].contains(&status), "bad status: {status}");
                count += 1;
            }
        }
        assert_eq!(count, 1000);

        let (cached, pinned) = cache.stats();
        assert!(pinned > 0, "should have pinned entries from dedup");
        assert!(cached > 0);
    }

    #[test]
    fn stress_dedup_savings_measurement() {
        let schema = Schema::new()
            .col("region", DType::Str)
            .col("service", DType::Str)
            .col("counter", DType::I64);

        let regions = ["us-east-1", "us-west-2", "eu-west-1", "ap-southeast-1"];
        let services = ["gateway", "auth", "billing", "inventory", "shipping"];
        let n = 500;

        // with dedup
        let size = MemTable::required_size(&schema, 65536, 1);
        let mut buf_dedup = vec![0u8; size];
        {
            let mut dw = DedupWriter::init(&mut buf_dedup, &schema, 65536, 1);
            for i in 0..n {
                dw.push_row(&[
                    Value::Str(regions[i % regions.len()]),
                    Value::Str(services[i % services.len()]),
                    Value::I64(i as i64),
                ]);
            }
        }
        let dedup_used = {
            let v = MemTableView::new(&buf_dedup).unwrap();
            v.chunk_used(0)
        };

        // without dedup
        let mut buf_plain = vec![0u8; size];
        {
            let mut mt = MemTableMut::init(&mut buf_plain, &schema, 65536, 1);
            for i in 0..n {
                mt.push_row(&[
                    Value::Str(regions[i % regions.len()]),
                    Value::Str(services[i % services.len()]),
                    Value::I64(i as i64),
                ]);
            }
        }
        let plain_used = {
            let v = MemTableView::new(&buf_plain).unwrap();
            v.chunk_used(0)
        };

        assert!(
            dedup_used < plain_used,
            "dedup should save space: {dedup_used} vs {plain_used}"
        );
        let savings_pct = (1.0 - dedup_used as f64 / plain_used as f64) * 100.0;
        assert!(
            savings_pct > 20.0,
            "expected >20% savings, got {savings_pct:.1}%"
        );

        // both should produce identical logical data
        let v_dedup = MemTableView::new(&buf_dedup).unwrap();
        let v_plain = MemTableView::new(&buf_plain).unwrap();
        assert_eq!(v_dedup.num_rows(0), v_plain.num_rows(0));
        for (rd, rp) in v_dedup.rows(0).zip(v_plain.rows(0)) {
            let mut cd = rd.cursor();
            let mut cp = rp.cursor();
            assert_eq!(cd.next_str(), cp.next_str());
            assert_eq!(cd.next_str(), cp.next_str());
            assert_eq!(cd.next_i64(), cp.next_i64());
        }
    }

    #[test]
    fn stress_many_columns_dedup() {
        let mut schema = Schema::new();
        for i in 0..16 {
            schema = schema.col(&format!("c{i}"), if i % 2 == 0 { DType::Str } else { DType::I32 });
        }
        let size = MemTable::required_size(&schema, 65536, 1);
        let mut buf = vec![0u8; size];
        let mut dw = DedupWriter::init(&mut buf, &schema, 65536, 1);

        let tags = ["x", "y", "z"];
        for i in 0..200 {
            let mut values: Vec<Value> = Vec::new();
            for col in 0..16 {
                if col % 2 == 0 {
                    values.push(Value::Str(tags[i % tags.len()]));
                } else {
                    values.push(Value::I32((i * 16 + col) as i32));
                }
            }
            dw.push_row(&values);
        }

        // verify all rows
        let mut count = 0;
        for row in dw.rows(0) {
            let mut c = row.cursor();
            for col in 0..16 {
                if col % 2 == 0 {
                    let s = c.next_str();
                    assert!(tags.contains(&s), "bad str at col {col}: {s}");
                } else {
                    let v = c.next_i32();
                    assert!(v >= 0);
                }
            }
            count += 1;
        }
        assert_eq!(count, 200);
    }

    #[test]
    fn stress_tiny_chunks_rapid_advance() {
        let schema = Schema::new().col("tag", DType::Str).col("v", DType::I32);
        // each chunk fits ~2-3 rows only
        let size = MemTable::required_size(&schema, 64, 16);
        let mut buf = vec![0u8; size];
        let mut dw = DedupWriter::init(&mut buf, &schema, 64, 16);

        let tags = ["aaa", "bbb"];
        for i in 0..200 {
            dw.push_row(&[Value::Str(tags[i % 2]), Value::I32(i as i32)]);
        }

        let mut total = 0;
        for chunk in 0..dw.num_chunks() {
            for row in dw.rows(chunk) {
                let mut c = row.cursor();
                let tag = c.next_str();
                let _v = c.next_i32();
                assert!(tags.contains(&tag));
                total += 1;
            }
        }
        assert!(total > 0, "should have rows across chunks");
    }

    #[test]
    fn stress_long_strings_dedup() {
        let schema = Schema::new().col("payload", DType::Str);
        let size = MemTable::required_size(&schema, 65536, 2);
        let mut buf = vec![0u8; size];
        let mut dw = DedupWriter::init(&mut buf, &schema, 65536, 2);

        // a 1KB string repeated many times
        let long_str: String = "x".repeat(1024);
        let short_str = "tiny";

        dw.push_row(&[Value::Str(&long_str)]);
        let after_first = dw.chunk_used(0);

        for _ in 0..50 {
            dw.push_row(&[Value::Str(&long_str)]);
        }
        let after_51 = dw.chunk_used(0);

        // 50 dedup refs should use ~50*8 = 400 bytes, not 50*1028
        let dedup_data = after_51 - after_first;
        assert!(dedup_data < 1000, "dedup of 1KB string should save space: {dedup_data}");

        dw.push_row(&[Value::Str(short_str)]);

        for row in dw.rows(0) {
            let s = row.col_str(0);
            assert!(s == long_str || s == short_str, "bad: len={}", s.len());
        }
    }

    #[test]
    fn stress_refcount_concurrent_dedup_lifecycle() {
        use std::alloc;
        use std::sync::{Arc, Barrier};
        use std::thread;

        let schema = Schema::new().col("tag", DType::Str).col("v", DType::I64);
        let size = MemTable::required_size(&schema, 16384, 4);
        let layout = alloc::Layout::from_size_align(size, 64).unwrap();
        let ptr = unsafe { alloc::alloc_zeroed(layout) };
        assert!(!ptr.is_null());

        unsafe {
            let buf = std::slice::from_raw_parts_mut(ptr, size);
            init_buf(buf, &schema, 16384, 4);
        }

        let num_producers = 4;
        let num_consumers = 4;
        let rows_per_producer = 200;
        let addr = ptr as usize;

        // acquire refs for all consumers
        unsafe {
            let buf = std::slice::from_raw_parts(ptr, size);
            for _ in 0..num_consumers {
                acquire_ref(buf);
            }
            assert_eq!(refcount(buf), 1 + num_consumers as u32);
        }

        let barrier = Arc::new(Barrier::new(num_producers));

        let producer_handles: Vec<_> = (0..num_producers)
            .map(|tid| {
                let barrier = barrier.clone();
                thread::spawn(move || {
                    barrier.wait();
                    let buf = unsafe { std::slice::from_raw_parts_mut(addr as *mut u8, size) };
                    let mut mt = MemTableMut::new(buf).unwrap();
                    for i in 0..rows_per_producer as i64 {
                        mt.push_row(&[Value::Str("tag"), Value::I64(tid as i64 * 10000 + i)]);
                    }
                })
            })
            .collect();

        for h in producer_handles {
            h.join().unwrap();
        }

        // consumers verify data and release refs
        let consumer_barrier = Arc::new(Barrier::new(num_consumers));
        let consumer_handles: Vec<_> = (0..num_consumers)
            .map(|_| {
                let barrier = consumer_barrier.clone();
                thread::spawn(move || {
                    barrier.wait();
                    let buf = unsafe { std::slice::from_raw_parts(addr as *const u8, size) };
                    let view = MemTableView::new(buf).unwrap();
                    let mut cache = CachedReader::new(buf, 64);
                    let mut count = 0;
                    for chunk in 0..view.num_chunks() {
                        for row in view.rows(chunk) {
                            let mut c = cache.cursor(&row);
                            let tag = c.next_str();
                            let v = c.next_i64();
                            assert_eq!(tag, "tag");
                            assert!(v >= 0);
                            count += 1;
                        }
                    }
                    assert_eq!(count, num_producers * rows_per_producer);
                    release_ref(buf);
                })
            })
            .collect();

        for h in consumer_handles {
            h.join().unwrap();
        }

        unsafe {
            let buf = std::slice::from_raw_parts(ptr, size);
            assert_eq!(refcount(buf), 1);
            release_ref(buf);
            alloc::dealloc(ptr as *mut u8, layout);
        }
    }

    // ── targeted tests for the 4 improvements ──────────────────────

    #[test]
    fn row_becomes_invalid_after_wrap_and_col_asserts() {
        let schema = Schema::new().col("v", DType::I64);
        let mut t = MemTable::new(&schema, 80, 2);
        t.push_row(&[Value::I64(1)]);
        let rows: Vec<_> = t.rows(0).collect();
        assert_eq!(rows[0].col_i64(0), 1);
        let gen_before = rows[0].generation();

        // wrap chunk 0 twice: 0→1→0 so chunk 0 gets a new generation
        t.advance_chunk();
        t.advance_chunk();
        let gen_after = t.chunk_generation(0);
        assert_ne!(gen_before, gen_after, "generation should have changed after wrap");
    }

    #[test]
    fn cached_reader_does_not_reuse_old_entry_after_generation_change() {
        let schema = Schema::new().col("s", DType::Str);
        let mut buf = vec![0u8; 4096];
        init_buf(&mut buf, &schema, 512, 2); // 2 chunks: wrap happens fast

        // Shared-memory style: reader holds &[u8] via raw pointer,
        // writer holds &mut [u8] — mirrors real cross-thread usage.
        let reader_buf: &[u8] = unsafe {
            std::slice::from_raw_parts(buf.as_ptr(), buf.len())
        };

        // Phase 1: write "hello" into chunk 0
        {
            let mut m = MemTableMut::new(&mut buf).unwrap();
            m.push_row(&[Value::Str("hello")]);
        }
        let gen0 = MemTableView::new(reader_buf).unwrap().chunk_generation(0);

        // Phase 2: read with cache — populates cache for chunk 0
        let mut cache = CachedReader::new(reader_buf, 64);
        let view = MemTableView::new(reader_buf).unwrap();
        for row in view.rows(0) {
            let mut c = cache.cursor(&row);
            assert_eq!(c.next_str(), "hello");
        }
        let (cache_sz_before, _) = cache.stats();
        assert!(cache_sz_before > 0);

        // Phase 3: advance twice to recycle chunk 0 (0→1→0), write "world"
        {
            let mut m = MemTableMut::new(&mut buf).unwrap();
            m.advance_chunk(); // 0→1
            m.advance_chunk(); // 1→0 (chunk 0 recycled, generation bumped)
            m.push_row(&[Value::Str("world")]);
        }
        let gen0_new = view.chunk_generation(0);
        assert_ne!(gen0, gen0_new);

        // Phase 4: read chunk 0 again — must see "world", not cached "hello"
        for row in view.rows(0) {
            let mut c = cache.cursor(&row);
            assert_eq!(c.next_str(), "world");
        }
    }

    #[test]
    fn from_buf_rejects_bad_version() {
        let schema = Schema::new().col("x", DType::U32);
        let mut t = MemTable::new(&schema, 256, 2);
        // Corrupt version field
        header_mut(t.as_bytes_mut()).version = 99;
        let raw = t.as_bytes().to_vec();
        assert!(MemTable::from_buf(raw).is_none());
    }

    #[test]
    fn from_buf_rejects_bad_data_offset() {
        let schema = Schema::new().col("x", DType::U32);
        let mut t = MemTable::new(&schema, 256, 2);
        // Corrupt data_offset
        header_mut(t.as_bytes_mut()).data_offset = 7;
        let raw = t.as_bytes().to_vec();
        assert!(MemTable::from_buf(raw).is_none());
    }

    #[test]
    #[should_panic(expected = "value types do not match schema")]
    fn push_row_rejects_wrong_column_count() {
        let schema = Schema::new().col("a", DType::U32).col("b", DType::I64);
        let mut t = MemTable::new(&schema, 256, 2);
        t.push_row(&[Value::U32(1)]); // only 1 value for 2 columns
    }

    #[test]
    #[should_panic(expected = "value types do not match schema")]
    fn push_row_rejects_wrong_dtype() {
        let schema = Schema::new().col("a", DType::U32);
        let mut t = MemTable::new(&schema, 256, 2);
        t.push_row(&[Value::Str("oops")]); // Str instead of U32
    }
}
