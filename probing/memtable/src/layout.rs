//! Low-level layout: header, column descriptors, chunk headers, byte helpers.
//!
//! ## Header v4 binary layout (64 bytes, 1 cache line)
//!
//! ```text
//! offset  size  field               notes
//! ──────────────────────────────────────────────────────────
//!  0       4    magic               0x4D454D54 ("MEMT" in LE)
//!  4       2    version             4
//!  6       2    header_size         64 (validation only)
//!  8       2    byte_order          BOM: written as [0x01, 0x02]
//! 10       2    ts_col              timestamp column index + 1 (0 = none)
//! 12       4    flags               feature bits (see FLAG_*)
//! 16       4    num_cols
//! 20       4    num_chunks
//! 24       4    chunk_size
//! 28       4    data_offset         (64-aligned)
//! ─── 32 byte boundary (cold/hot split) ─────────────────
//! 32       4    write_chunk         AtomicU32
//! 36       4    refcount            AtomicU32
//! 40       4    creator_pid         PID of creating process
//! 44       4    _pad0               (alignment)
//! 48       8    creator_start_time  process start time (platform-specific)
//! 56       8    _reserved           reserved for future use
//! ──────────────────────────────────────────────────────────
//! ```
//!
//! All multi-byte fields are little-endian.  The `byte_order` BOM
//! allows readers to detect endianness mismatch without guessing.
//!
//! MEMT is **single-writer**: exactly one writer owns each buffer (the
//! creator process; in-process writes are serialised by the caller). There
//! is no in-buffer write lock. Readers stay lock-free via per-chunk
//! `generation` re-validation.

use std::mem;
use std::sync::atomic::{AtomicI64, AtomicU32, AtomicU64};

// ── C-style layout structs ──────────────────────────────────────────

/// Magic number for MEMT (ring-buffer time-series table): bytes `M E M T` in little-endian.
pub const MAGIC_MEMT: u32 = 0x4D45_4D54;
pub(crate) const MAGIC: u32 = MAGIC_MEMT;

/// Header format version for MEMT.
///
/// v4: dropped the `write_lock` and `lock_owner_start` fields — MEMT is
/// single-writer, so there is no in-buffer write lock. Their bytes are now
/// `_pad0`/`_reserved`. v3 added per-chunk `min_ts`/`max_ts` and the PID
/// write lock (both since superseded).
pub(crate) const VERSION: u16 = 4;

/// Byte-order mark: written as raw bytes `[0x01, 0x02]`.
/// On a LE host, `u16::from_ne_bytes([0x01, 0x02])` == `0x0201`.
pub(crate) const BYTE_ORDER_MARK: [u8; 2] = [0x01, 0x02];

/// Feature flag: dedup back-references may appear in Str/Bytes columns.
///
/// Set when dedup is enabled.  When absent, `validate_buf`
/// rejects any negative length-prefix (dedup ref) as invalid.
pub(crate) const FLAG_DEDUP: u32 = 1 << 0;
// Reserved for future use:
// pub const FLAG_CHECKSUM:  u32 = 1 << 1;
// pub const FLAG_COMPRESSED: u32 = 1 << 2;
// pub const FLAG_SORTED:    u32 = 1 << 3;

/// Bits that this version of the library understands.
pub(crate) const FLAGS_KNOWN: u32 = FLAG_DEDUP;

/// Fixed header at the start of every MemTable buffer (64 bytes).
///
/// **Cold zone** (bytes 0–31): immutable after init — `magic`, `version`,
/// schema dimensions, layout offsets.
///
/// **Hot zone** (bytes 32–63): atomically mutated at runtime —
/// `write_chunk`, `refcount`.  Separated from the cold zone to avoid
/// false-sharing on different cache lines.
#[repr(C)]
pub(crate) struct Header {
    // ── cold zone (read-only after init) ─────────────────
    pub magic: u32,
    pub version: u16,
    /// Size of this header in bytes (always 64 in v2).
    ///
    /// Used for validation only — column descriptors always start at
    /// offset `size_of::<Header>()` (compile-time constant).  If a
    /// future version extends the header, it will bump `version` and
    /// `header_size` together so that older readers can detect the
    /// mismatch and reject the buffer cleanly.
    pub header_size: u16,
    /// Byte-order mark, written as `BYTE_ORDER_MARK`.
    pub byte_order: u16,
    /// Designated timestamp column **index + 1** (0 = no timestamp column).
    ///
    /// Set at init when the schema contains an `I64` column named
    /// `"timestamp"`. The writer maintains per-chunk `min_ts`/`max_ts`
    /// from this column so readers can prune chunks by time range.
    pub ts_col: u16,
    /// Feature flags (see `FLAG_*` constants).
    pub flags: u32,
    pub num_cols: u32,
    pub num_chunks: u32,
    pub chunk_size: u32,
    /// Byte offset where chunk data begins (64-aligned).
    pub data_offset: u32,

    // ── hot zone (atomically mutated) ────────────────────
    /// Ring buffer: index of the chunk currently being written.
    pub write_chunk: AtomicU32,
    /// Reference count for shared lifetime management.
    pub refcount: AtomicU32,
    /// PID of the process that created this table (for cross-process discovery).
    pub creator_pid: u32,
    /// Padding to 8-align `creator_start_time` (was `write_lock` in v3).
    pub _pad0: u32,
    /// Process start time — for PID-recycling detection during discovery.
    /// Linux: clock ticks since boot (`/proc/<pid>/stat` field 22).
    /// macOS: microseconds since epoch (via `sysctl`).
    /// Other: 0 (falls back to PID-only liveness check).
    pub creator_start_time: u64,
    /// Reserved for future use (was `lock_owner_start` in v3).
    pub _reserved: u64,
}

/// Per-column descriptor, immediately following the Header.
#[repr(C)]
pub(crate) struct ColumnDesc {
    /// Column name, length-prefixed: `[u16 len][utf8 bytes][padding]`.
    pub name: [u8; 56],
    /// `DType` value as `u32`.
    pub dtype: u32,
    /// For fixed-size types: byte size. For `Str`/`Bytes`: 0 (variable-length).
    pub elem_size: u32,
}

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

/// Sentinel for `ChunkHeader.min_ts` when the chunk holds no rows.
pub(crate) const TS_MIN_INIT: i64 = i64::MAX;
/// Sentinel for `ChunkHeader.max_ts` when the chunk holds no rows.
pub(crate) const TS_MAX_INIT: i64 = i64::MIN;

/// Per-chunk metadata, at the start of every chunk's byte region (40 bytes).
#[repr(C)]
pub(crate) struct ChunkHeader {
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
    /// Smallest value of the designated timestamp column in this chunk
    /// ([`TS_MIN_INIT`] when empty or no `Header::ts_col`). Maintained by
    /// the writer; readers must validate against `generation` snapshots.
    pub min_ts: AtomicI64,
    /// Largest timestamp in this chunk ([`TS_MAX_INIT`] when empty).
    pub max_ts: AtomicI64,
}

/// Chunk lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub(crate) enum ChunkState {
    Empty = 0,
    Writing = 1,
    Sealed = 2,
}

pub(crate) const CHUNK_HEADER_SIZE: usize = mem::size_of::<ChunkHeader>();

const _: () = {
    assert!(mem::size_of::<Header>() == 64);
    assert!(mem::size_of::<ColumnDesc>() == 64);
    assert!(mem::size_of::<ChunkHeader>() == 40);
};
// ── struct accessors ────────────────────────────────────────────────

pub(crate) fn header(buf: &[u8]) -> &Header {
    debug_assert!(buf.len() >= mem::size_of::<Header>());
    unsafe { &*(buf.as_ptr() as *const Header) }
}

pub(crate) fn header_mut(buf: &mut [u8]) -> &mut Header {
    debug_assert!(buf.len() >= mem::size_of::<Header>());
    unsafe { &mut *(buf.as_mut_ptr() as *mut Header) }
}

pub(crate) fn col_desc(buf: &[u8], col: usize) -> &ColumnDesc {
    let off = mem::size_of::<Header>() + col * mem::size_of::<ColumnDesc>();
    debug_assert!(buf.len() >= off + mem::size_of::<ColumnDesc>());
    unsafe { &*(buf[off..].as_ptr() as *const ColumnDesc) }
}

pub(crate) fn col_desc_mut(buf: &mut [u8], col: usize) -> &mut ColumnDesc {
    let off = mem::size_of::<Header>() + col * mem::size_of::<ColumnDesc>();
    debug_assert!(buf.len() >= off + mem::size_of::<ColumnDesc>());
    unsafe { &mut *(buf[off..].as_mut_ptr() as *mut ColumnDesc) }
}

// ── chunk header accessor ───────────────────────────────────────────

pub(crate) fn chunk_header(buf: &[u8], cs: usize) -> &ChunkHeader {
    debug_assert!(cs % 8 == 0 && buf.len() >= cs + CHUNK_HEADER_SIZE);
    unsafe { &*(buf[cs..].as_ptr() as *const ChunkHeader) }
}

pub(crate) fn r32(buf: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(buf[off..off + 4].try_into().unwrap())
}

pub(crate) fn w32(buf: &mut [u8], off: usize, v: u32) {
    buf[off..off + 4].copy_from_slice(&v.to_le_bytes());
}

pub(crate) fn align64(n: usize) -> usize {
    (n + 63) & !63
}

// ── layout helpers ──────────────────────────────────────────────────

pub(crate) fn compute_data_offset(num_cols: usize) -> usize {
    align64(mem::size_of::<Header>() + num_cols * mem::size_of::<ColumnDesc>())
}

pub(crate) fn chunk_start_off(buf: &[u8], chunk: usize) -> usize {
    let h = header(buf);
    h.data_offset as usize + chunk * h.chunk_size as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem;

    #[test]
    fn struct_sizes() {
        assert_eq!(mem::size_of::<Header>(), 64);
        assert_eq!(mem::size_of::<ColumnDesc>(), 64);
        assert_eq!(mem::size_of::<ChunkHeader>(), 40);
    }

    #[test]
    fn byte_order_mark_sanity() {
        let bom = u16::from_ne_bytes(BYTE_ORDER_MARK);
        let expected_le = u16::from_le_bytes(BYTE_ORDER_MARK);
        assert_eq!(bom, expected_le);
    }
}
