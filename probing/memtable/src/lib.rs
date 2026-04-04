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

mod buf;
mod cache;
mod dedup;
mod layout;
mod memtable;
mod refcount;
mod row;
mod schema;
mod table;
mod value;
mod writer;

pub use buf::validate_buf;
pub use cache::{CachedCursor, CachedReader};
pub use dedup::DedupState;
pub use layout::{ChunkHeader, ChunkState, ColumnDesc, Header, MAGIC, VERSION};
pub use memtable::{DedupWriter, MemTable, MemTableMut, MemTableView};
pub use refcount::{acquire_ref, refcount, release_ref};
pub use row::{Row, RowCursor, RowIter};
pub use schema::{Col, DType, Schema};
pub use value::Value;
pub use writer::{DedupRowWriter, RowWriter};
