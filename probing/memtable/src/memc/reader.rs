//! [`SegmentReader`]: mmap a `.memc` file and read its tables and pages.
//!
//! A sealed segment is read via its footer page directory. An unsealed or
//! torn segment (writer crashed before `seal`) falls back to a forward
//! scan of checksummed blocks, stopping at the first damaged/partial block
//! — so a half-written tail is silently dropped rather than surfaced.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

use memmap2::Mmap;

use super::codec::{decode_column, ColumnData};
use super::layout::{
    get_u32, xxh32, BlockHeader, SegmentHeader, TableDef, BLOCK_HEADER_SIZE, MAGIC_FOOTER,
    MAGIC_PAGE_BLOCK, MAGIC_TABLE_BLOCK, PAGE_DIR_ENTRY_SIZE, SEGMENT_HEADER_SIZE,
};

/// Metadata for one page, enough to prune before decoding.
#[derive(Debug, Clone)]
pub struct PageMeta {
    pub table_id: u32,
    pub row_count: u32,
    pub col_count: u32,
    pub ts_min: i64,
    pub ts_max: i64,
    pub block_off: u64,
    pub block_len: u32,
    pub source_gen: u64,
    pub source_chunk: u32,
    pub source_instance: u64,
}

/// Read-only view over a memory-mapped MEMC segment.
pub struct SegmentReader {
    mmap: Mmap,
    path: PathBuf,
    header: SegmentHeader,
    tables: HashMap<u32, TableDef>,
    pages: Vec<PageMeta>,
}

impl SegmentReader {
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let file = std::fs::File::open(&path)?;
        let mmap = unsafe { Mmap::map(&file)? };
        Self::from_mmap(mmap, path)
    }

    fn from_mmap(mmap: Mmap, path: PathBuf) -> io::Result<Self> {
        let header = SegmentHeader::decode(&mmap)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        let mut tables = HashMap::new();
        let mut pages = Vec::new();

        let footer_ok = header.is_sealed()
            && header.footer_off != 0
            && Self::load_footer(&mmap, &header, &mut pages);

        // Always scan blocks for table definitions (cheap; MCTB blocks live
        // before pages). On footer failure this also recovers page metadata.
        Self::scan_blocks(&mmap, &header, &mut tables, footer_ok, &mut pages);

        Ok(Self {
            mmap,
            path,
            header,
            tables,
            pages,
        })
    }

    /// Parse the footer page directory. Returns `false` (and leaves `pages`
    /// untouched) if the footer is malformed or fails its checksum.
    fn load_footer(mmap: &[u8], header: &SegmentHeader, pages: &mut Vec<PageMeta>) -> bool {
        let Ok(foff) = usize::try_from(header.footer_off) else {
            return false;
        };
        let Some(footer_header_end) = foff.checked_add(16) else {
            return false;
        };
        if footer_header_end > mmap.len() || get_u32(mmap, foff) != MAGIC_FOOTER {
            return false;
        }
        let count = get_u32(mmap, foff + 4) as usize;
        let entries_len = get_u32(mmap, foff + 8) as usize;
        let checksum = get_u32(mmap, foff + 12);
        let Some(expected_entries_len) = count.checked_mul(PAGE_DIR_ENTRY_SIZE) else {
            return false;
        };
        if count != header.page_count as usize || entries_len != expected_entries_len {
            return false;
        }
        let entries_start = footer_header_end;
        let Some(entries_end) = entries_start.checked_add(entries_len) else {
            return false;
        };
        if entries_end > mmap.len() || xxh32(&mmap[entries_start..entries_end]) != checksum {
            return false;
        }

        let mut out = Vec::with_capacity(count);
        for i in 0..count {
            let Some(entry_off) = i
                .checked_mul(PAGE_DIR_ENTRY_SIZE)
                .and_then(|offset| entries_start.checked_add(offset))
            else {
                return false;
            };
            let block_off = super::layout::get_u64(mmap, entry_off + 24);
            let block_len = get_u32(mmap, entry_off + 32);
            let Ok(block_start) = usize::try_from(block_off) else {
                return false;
            };
            let Some(block_end) = block_start.checked_add(block_len as usize) else {
                return false;
            };
            if block_start < SEGMENT_HEADER_SIZE
                || block_len < BLOCK_HEADER_SIZE as u32
                || block_end > foff
            {
                return false;
            }
            out.push(PageMeta {
                table_id: get_u32(mmap, entry_off),
                row_count: get_u32(mmap, entry_off + 4),
                ts_min: super::layout::get_i64(mmap, entry_off + 8),
                ts_max: super::layout::get_i64(mmap, entry_off + 16),
                block_off,
                block_len,
                col_count: get_u32(mmap, entry_off + 36),
                source_gen: super::layout::get_u64(mmap, entry_off + 40),
                source_chunk: get_u32(mmap, entry_off + 48),
                source_instance: super::layout::get_u64(mmap, entry_off + 56),
            });
        }
        *pages = out;
        true
    }

    /// Forward-scan blocks from the first block to `footer_off`/EOF.
    /// Collects table definitions always; collects page metadata only when
    /// `footer_ok` is false (recovery path). Stops at the first block that
    /// fails to decode or whose payload checksum mismatches.
    fn scan_blocks(
        mmap: &[u8],
        header: &SegmentHeader,
        tables: &mut HashMap<u32, TableDef>,
        footer_ok: bool,
        pages: &mut Vec<PageMeta>,
    ) {
        let limit = if header.footer_off != 0 {
            usize::try_from(header.footer_off)
                .unwrap_or(usize::MAX)
                .min(mmap.len())
        } else {
            mmap.len()
        };
        let mut off = SEGMENT_HEADER_SIZE;
        while off
            .checked_add(BLOCK_HEADER_SIZE)
            .is_some_and(|end| end <= limit)
        {
            let Some(bh) = BlockHeader::decode(&mmap[off..]) else {
                break;
            };
            let Some(payload_start) = off.checked_add(BLOCK_HEADER_SIZE) else {
                break;
            };
            let Some(payload_end) = payload_start.checked_add(bh.payload_len as usize) else {
                break;
            };
            if payload_end > limit {
                break; // torn tail
            }
            if xxh32(&mmap[payload_start..payload_end]) != bh.payload_xxh {
                break; // corrupt payload — stop here
            }
            let Some(raw_block_len) = BLOCK_HEADER_SIZE.checked_add(bh.payload_len as usize) else {
                break;
            };
            let Some(block_len) = raw_block_len.checked_add(63).map(|n| n & !63) else {
                break;
            };
            let Some(next_off) = off.checked_add(block_len) else {
                break;
            };
            if next_off > limit {
                break;
            }

            match bh.magic {
                MAGIC_TABLE_BLOCK => {
                    if let Ok(def) = super::layout::decode_table_payload(
                        bh.table_id,
                        &mmap[payload_start..payload_end],
                    ) {
                        tables.insert(bh.table_id, def);
                    }
                }
                MAGIC_PAGE_BLOCK if !footer_ok => {
                    pages.push(PageMeta {
                        table_id: bh.table_id,
                        row_count: bh.row_count,
                        col_count: bh.col_count,
                        ts_min: bh.ts_min,
                        ts_max: bh.ts_max,
                        block_off: off as u64,
                        block_len: block_len as u32,
                        source_gen: bh.source_gen,
                        source_chunk: bh.source_chunk,
                        source_instance: bh.source_instance,
                    });
                }
                _ => {}
            }
            off = next_off;
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn is_sealed(&self) -> bool {
        self.header.is_sealed()
    }

    /// Segment-wide timestamp range (sealed segments only; `None` otherwise
    /// or when the segment has no timestamped rows).
    pub fn ts_range(&self) -> Option<(i64, i64)> {
        if self.header.is_sealed() && self.header.ts_min <= self.header.ts_max {
            Some((self.header.ts_min, self.header.ts_max))
        } else {
            None
        }
    }

    pub fn table_defs(&self) -> Vec<&TableDef> {
        self.tables.values().collect()
    }

    pub fn table_def(&self, id: u32) -> Option<&TableDef> {
        self.tables.get(&id)
    }

    pub fn table_id_by_name(&self, name: &str) -> Option<u32> {
        self.tables.values().find(|d| d.name == name).map(|d| d.id)
    }

    pub fn pages(&self) -> &[PageMeta] {
        &self.pages
    }

    /// Pages for `table_id` whose `[ts_min, ts_max]` overlaps `[lo, hi]`
    /// (either bound `None` = unbounded). Pages without a ts range
    /// (`ts_min > ts_max`) are always included.
    pub fn pages_in_range(&self, table_id: u32, lo: Option<i64>, hi: Option<i64>) -> Vec<usize> {
        self.pages
            .iter()
            .enumerate()
            .filter(|(_, p)| p.table_id == table_id)
            .filter(|(_, p)| {
                if p.ts_min > p.ts_max {
                    return true; // no ts metadata: cannot prune
                }
                !(lo.is_some_and(|l| p.ts_max < l) || hi.is_some_and(|h| p.ts_min > h))
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// Decode page `index` into its columns (in schema order).
    pub fn read_page(&self, index: usize) -> Result<Vec<ColumnData>, String> {
        let p = self.pages.get(index).ok_or("page index out of range")?;
        let hstart =
            usize::try_from(p.block_off).map_err(|_| "page block offset does not fit usize")?;
        let header_end = hstart
            .checked_add(BLOCK_HEADER_SIZE)
            .ok_or("page block header offset overflow")?;
        if header_end > self.mmap.len() {
            return Err("page block header out of bounds".into());
        }
        let bh = BlockHeader::decode(&self.mmap[hstart..]).ok_or("page block header invalid")?;
        if bh.magic != MAGIC_PAGE_BLOCK {
            return Err("page directory points to a non-page block".into());
        }
        let raw_block_len = BLOCK_HEADER_SIZE
            .checked_add(bh.payload_len as usize)
            .ok_or("page block length overflow")?;
        let block_len = raw_block_len
            .checked_add(63)
            .map(|n| n & !63)
            .ok_or("page aligned block length overflow")?;
        if bh.table_id != p.table_id
            || bh.row_count != p.row_count
            || bh.col_count != p.col_count
            || block_len != p.block_len as usize
        {
            return Err("page directory does not match block header".into());
        }
        let payload_start = header_end;
        let payload_end = payload_start
            .checked_add(bh.payload_len as usize)
            .ok_or("page payload offset overflow")?;
        if payload_end > self.mmap.len() {
            return Err("page payload out of bounds".into());
        }
        if xxh32(&self.mmap[payload_start..payload_end]) != bh.payload_xxh {
            return Err("page payload checksum mismatch".into());
        }

        let rc = bh.row_count as usize;
        let mut cols = Vec::with_capacity(bh.col_count as usize);
        let mut off = payload_start;
        for _ in 0..bh.col_count {
            let (col, used) = decode_column(&self.mmap[off..payload_end], rc)?;
            if col.len() != rc {
                return Err(format!(
                    "decoded column row count mismatch: expected {rc}, got {}",
                    col.len()
                ));
            }
            cols.push(col);
            off = off.checked_add(used).ok_or("column offset overflow")?;
        }
        if off != payload_end {
            return Err("page payload has trailing bytes".into());
        }
        Ok(cols)
    }
}
