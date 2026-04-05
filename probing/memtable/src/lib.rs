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
//! See [`layout`] module for the full binary specification.
//!
//! ```text
//! ┌──────────────────────────────────┐ 0
//! │ Header v2 (64 bytes, repr(C))    │
//! │  ── cold zone (read-only) ──     │
//! │   magic: u32     (0x4D454D54)    │
//! │   version: u16   (2)             │
//! │   header_size: u16 (64)          │
//! │   byte_order: u16 (BOM 0x0102)   │
//! │   _pad0: u16                     │
//! │   flags: u32     (feature bits)  │
//! │   num_cols: u32                  │
//! │   num_chunks: u32                │
//! │   chunk_size: u32                │
//! │   data_offset: u32               │
//! │  ── hot zone (atomic) ────       │
//! │   write_chunk: AtomicU32         │
//! │   write_lock: AtomicU32          │
//! │   refcount: AtomicU32            │
//! │   creator_pid: u32                │
//! │   creator_start_time: u64         │
//! │   _reserved: [u32; 2]            │
//! ├──────────────────────────────────┤ 64
//! │ ColumnDesc × N (64 bytes each)   │
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

mod cache;
mod dedup;
pub mod discover;
mod layout;
mod memtable;
mod raw;
mod refcount;
mod row;
mod schema;
mod writer;

pub use cache::{CachedCursor, CachedReader};
pub use memtable::{MemTable, MemTableView, MemTableWriter};
pub use raw::validate_buf;
pub use refcount::{acquire_ref, refcount, release_ref};
pub use row::{Row, RowCursor, RowIter};
pub use schema::{Col, DType, Schema, Value};
pub use writer::RowWriter;
