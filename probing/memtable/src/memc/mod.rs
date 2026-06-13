//! MEMC: **cold** columnar segment files — the on-disk second tier below
//! the hot MEMT ring.
//!
//! A background compactor drains sealed chunks from a hot [`MemTable`] and
//! appends them, transposed to columns and Pco-compressed, as immutable
//! **pages** inside append-only **segment** files. Segments live in a
//! [`ColdStore`] directory and are evicted oldest-first by byte budget or
//! TTL — a second-level ring that gives the system a time-retention axis
//! the fixed-capacity hot ring cannot provide on its own.
//!
//! [`MemTable`]: crate::MemTable
//!
//! ## File format (one `.memc` segment)
//!
//! ```text
//! ┌────────────────────────────────────────────┐ 0
//! │ SegmentHeader (64 B)                         │
//! │   magic "MEMC", version, BOM, flags          │
//! │   writer pid/start, created_unix_ms          │
//! │   footer_off, ts_min/ts_max, page_count      │
//! │   header_xxh                                 │
//! ├────────────────────────────────────────────┤ 64
//! │ MCTB table-def block(s) — one per table      │
//! │   [BlockHeader 64B][name+columns payload]    │
//! ├────────────────────────────────────────────┤
//! │ MCPG page block(s) — columnar, multi-table   │
//! │   [BlockHeader 64B]                           │
//! │   per column: [enc][dtype][len][bytes]       │
//! │     numeric → Pco · u8/str/bytes → raw       │
//! ├────────────────────────────────────────────┤ footer_off
//! │ Footer: [MAGIC][count][len][xxh]             │
//! │   page directory: N × 48B                    │
//! │     (table_id, ts_min/max, block_off/len, …) │
//! └────────────────────────────────────────────┘
//! ```
//!
//! Every block header and payload carries an xxh3 checksum. Sealed
//! segments are read through the footer directory; if the writer crashed
//! before sealing, [`SegmentReader`] forward-scans the checksummed blocks
//! and drops the torn tail.
//!
//! ## Query path (two-level time pruning)
//!
//! Segment header `ts_min/ts_max` prunes whole files (no mmap), then the
//! page directory's per-page `(table_id, ts_min, ts_max)` prunes pages
//! before decode — mirroring the hot ring's chunk-level pruning so a query
//! planner can span hot chunks and cold pages with one time predicate.

mod codec;
mod compactor;
mod layout;
mod reader;
mod store;
mod writer;

pub use codec::{ColumnBuilder, ColumnData};
pub use compactor::{Compactor, CompactorConfig, CompactorHandle};
pub use layout::{ColEncoding, TableDef, MAGIC_MEMC, SOURCE_CHUNK_NONE, VERSION_MEMC};
pub use reader::{PageMeta, SegmentReader};
pub use store::{default_cold_dir, writer_id, ColdStats, ColdStore};
pub use writer::SegmentWriter;

#[cfg(test)]
mod tests;
