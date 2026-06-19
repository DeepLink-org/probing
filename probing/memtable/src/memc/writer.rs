//! [`SegmentWriter`]: build one `.memc` segment file incrementally.
//!
//! Lifecycle: create → `register_table`* → `append_page`* → `seal`.
//! Blocks are flushed to the file as they are produced; the footer (page
//! directory) and the sealed segment header are written last, so a crash
//! before `seal` leaves a forward-scannable, checksummed prefix.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{self, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use super::codec::{encode_column, ColumnData};
use super::layout::{
    align64, xxh32, BlockHeader, ColEncoding, SegmentHeader, BLOCK_HEADER_SIZE, FLAG_SEALED,
    MAGIC_FOOTER, MAGIC_PAGE_BLOCK, MAGIC_TABLE_BLOCK, PAGE_DIR_ENTRY_SIZE, SEGMENT_HEADER_SIZE,
    SOURCE_CHUNK_NONE, TS_MAX_INIT, TS_MIN_INIT,
};
use crate::raw::process_start_time;
use crate::schema::DType;

/// One page-directory entry, mirrored into the footer on seal.
#[derive(Debug, Clone)]
pub(crate) struct PageDirEntry {
    pub table_id: u32,
    pub row_count: u32,
    pub col_count: u32,
    pub ts_min: i64,
    pub ts_max: i64,
    pub block_off: u64,
    pub block_len: u32,
    pub source_gen: u64,
    pub source_chunk: u32,
}

struct TableInfo {
    cols: Vec<(String, DType)>,
    ts_col: Option<usize>,
}

/// Incremental writer for a single MEMC segment file.
pub struct SegmentWriter {
    file: File,
    path: PathBuf,
    offset: u64,
    tables: HashMap<u32, TableInfo>,
    next_table_id: u32,
    pages: Vec<PageDirEntry>,
    seg_ts_min: i64,
    seg_ts_max: i64,
    sealed: bool,
}

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl SegmentWriter {
    /// Create a new segment file at `path`, writing the (unsealed) header.
    pub fn create(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)?;

        let pid = std::process::id();
        let header = SegmentHeader {
            flags: 0,
            writer_pid: pid,
            writer_start: process_start_time(pid),
            created_unix_ms: now_unix_ms(),
            footer_off: 0,
            ts_min: TS_MIN_INIT,
            ts_max: TS_MAX_INIT,
            page_count: 0,
        };
        file.write_all(&header.encode())?;

        Ok(Self {
            file,
            path,
            offset: SEGMENT_HEADER_SIZE as u64,
            tables: HashMap::new(),
            next_table_id: 1,
            pages: Vec::new(),
            seg_ts_min: TS_MIN_INIT,
            seg_ts_max: TS_MAX_INIT,
            sealed: false,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    /// Bytes written to the segment so far (header + all blocks, before the
    /// footer). A compactor polls this to decide when to seal and roll to a
    /// fresh segment, bounding file size and preventing fragmentation.
    pub fn size_bytes(&self) -> u64 {
        self.offset
    }

    /// Timestamp span covered so far, `None` until a timestamped page lands.
    /// Lets a compactor also roll on a wall-clock window (e.g. seal every
    /// 5 min) so low-rate tables don't sit unsealed indefinitely.
    pub fn ts_span(&self) -> Option<(i64, i64)> {
        if self.seg_ts_min <= self.seg_ts_max {
            Some((self.seg_ts_min, self.seg_ts_max))
        } else {
            None
        }
    }

    /// Register a table, write its `MCTB` definition block, return its id.
    pub fn register_table(&mut self, name: &str, cols: &[(String, DType)]) -> io::Result<u32> {
        let id = self.next_table_id;
        self.next_table_id += 1;

        let payload = super::layout::encode_table_payload(name, cols);
        let header = BlockHeader {
            magic: MAGIC_TABLE_BLOCK,
            table_id: id,
            row_count: 0,
            col_count: cols.len() as u32,
            ts_min: TS_MIN_INIT,
            ts_max: TS_MAX_INIT,
            source_gen: 0,
            payload_len: payload.len() as u32,
            payload_xxh: xxh32(&payload),
            source_chunk: SOURCE_CHUNK_NONE,
        };
        self.write_block(&header, &payload)?;

        let ts_col = cols.iter().position(|(n, dt)| {
            *dt == DType::I64 && crate::raw::TS_COL_NAMES.contains(&n.as_str())
        });
        self.tables.insert(
            id,
            TableInfo {
                cols: cols.to_vec(),
                ts_col,
            },
        );
        Ok(id)
    }

    /// Append a columnar page for `table_id`. `source_gen` / `source_chunk`
    /// record the hot-ring chunk this page was compacted from (generation and
    /// chunk index); pass `(0, SOURCE_CHUNK_NONE)` when not applicable. They
    /// let a restarting compactor rebuild its per-chunk drain watermark.
    ///
    /// All columns must share the same length and match the registered
    /// schema's dtypes in order.
    pub fn append_page(
        &mut self,
        table_id: u32,
        columns: &[ColumnData],
        source_gen: u64,
        source_chunk: u32,
    ) -> io::Result<()> {
        let info = self
            .tables
            .get(&table_id)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "unknown table_id"))?;
        if columns.len() != info.cols.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "page column count mismatch",
            ));
        }
        let row_count = columns.first().map(|c| c.len()).unwrap_or(0);
        for (i, col) in columns.iter().enumerate() {
            if col.dtype() != info.cols[i].1 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "page column dtype mismatch",
                ));
            }
            if col.len() != row_count {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "page columns have unequal lengths",
                ));
            }
        }
        if row_count == 0 {
            return Ok(()); // nothing to persist
        }

        let (ts_min, ts_max) = match info.ts_col {
            Some(ci) => match &columns[ci] {
                ColumnData::I64(v) => v.iter().fold((TS_MIN_INIT, TS_MAX_INIT), |(lo, hi), &t| {
                    (lo.min(t), hi.max(t))
                }),
                _ => (TS_MIN_INIT, TS_MAX_INIT),
            },
            None => (TS_MIN_INIT, TS_MAX_INIT),
        };

        let mut payload = Vec::new();
        for col in columns {
            let sub = encode_column(col).map_err(|e| {
                io::Error::new(io::ErrorKind::InvalidData, format!("column encode: {e}"))
            })?;
            payload.extend_from_slice(&sub);
        }

        let header = BlockHeader {
            magic: MAGIC_PAGE_BLOCK,
            table_id,
            row_count: row_count as u32,
            col_count: columns.len() as u32,
            ts_min,
            ts_max,
            source_gen,
            payload_len: payload.len() as u32,
            payload_xxh: xxh32(&payload),
            source_chunk,
        };
        let block_off = self.offset;
        let block_len = self.write_block(&header, &payload)?;

        if ts_min <= ts_max {
            self.seg_ts_min = self.seg_ts_min.min(ts_min);
            self.seg_ts_max = self.seg_ts_max.max(ts_max);
        }
        self.pages.push(PageDirEntry {
            table_id,
            row_count: row_count as u32,
            col_count: columns.len() as u32,
            ts_min,
            ts_max,
            block_off,
            block_len: block_len as u32,
            source_gen,
            source_chunk,
        });
        Ok(())
    }

    /// Write the footer (page directory) and the sealed header, then flush.
    ///
    /// After this the file is immutable; the writer is consumed.
    pub fn seal(mut self) -> io::Result<PathBuf> {
        let footer_off = self.offset;
        let mut footer = Vec::with_capacity(16 + self.pages.len() * PAGE_DIR_ENTRY_SIZE);
        footer.extend_from_slice(&MAGIC_FOOTER.to_le_bytes());
        footer.extend_from_slice(&(self.pages.len() as u32).to_le_bytes());
        let entries_len = (self.pages.len() * PAGE_DIR_ENTRY_SIZE) as u32;
        footer.extend_from_slice(&entries_len.to_le_bytes());
        footer.extend_from_slice(&[0u8; 4]); // checksum placeholder

        let entries_start = footer.len();
        for p in &self.pages {
            footer.extend_from_slice(&p.table_id.to_le_bytes());
            footer.extend_from_slice(&p.row_count.to_le_bytes());
            footer.extend_from_slice(&p.ts_min.to_le_bytes());
            footer.extend_from_slice(&p.ts_max.to_le_bytes());
            footer.extend_from_slice(&p.block_off.to_le_bytes());
            footer.extend_from_slice(&p.block_len.to_le_bytes());
            footer.extend_from_slice(&p.col_count.to_le_bytes());
            footer.extend_from_slice(&p.source_gen.to_le_bytes());
            footer.extend_from_slice(&p.source_chunk.to_le_bytes());
            footer.extend_from_slice(&[0u8; 4]); // pad to 56
        }
        let checksum = xxh32(&footer[entries_start..]);
        footer[12..16].copy_from_slice(&checksum.to_le_bytes());

        self.file.write_all(&footer)?;

        // Rewrite the header with seal metadata.
        let pid = std::process::id();
        let header = SegmentHeader {
            flags: FLAG_SEALED,
            writer_pid: pid,
            writer_start: process_start_time(pid),
            created_unix_ms: now_unix_ms(),
            footer_off,
            ts_min: self.seg_ts_min,
            ts_max: self.seg_ts_max,
            page_count: self.pages.len() as u32,
        };
        self.file.seek(SeekFrom::Start(0))?;
        self.file.write_all(&header.encode())?;
        self.file.flush()?;
        self.file.sync_data()?;
        self.sealed = true;
        Ok(self.path.clone())
    }

    /// Write a block header + payload, zero-padded to a 64-byte boundary.
    /// Returns the total bytes written (the block length).
    fn write_block(&mut self, header: &BlockHeader, payload: &[u8]) -> io::Result<u64> {
        debug_assert!(matches!(
            ColEncoding::from_u8(0),
            Some(ColEncoding::RawFixed)
        ));
        let raw = BLOCK_HEADER_SIZE + payload.len();
        let padded = align64(raw);
        self.file.write_all(&header.encode())?;
        self.file.write_all(payload)?;
        if padded > raw {
            self.file.write_all(&vec![0u8; padded - raw])?;
        }
        self.offset += padded as u64;
        Ok(padded as u64)
    }
}

impl Drop for SegmentWriter {
    fn drop(&mut self) {
        // An unsealed segment on drop keeps its checksummed block prefix on
        // disk; the reader's forward-scan recovery path will pick it up.
        if !self.sealed {
            let _ = self.file.flush();
        }
    }
}
