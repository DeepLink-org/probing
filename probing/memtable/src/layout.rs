//! Low-level layout: header, column descriptors, chunk headers, byte helpers.
//!
//! ## Header v5 binary layout (128 bytes, 2 cache lines)
//!
//! ```text
//! offset  size  field               notes
//! ──────────────────────────────────────────────────────────
//!  0       4    magic               0x4D454D54 ("MEMT" in LE)
//!  4       2    version             5
//!  6       2    header_size         128 (validation only)
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
//! 56       4    chunks_recycled     AtomicU32 — ring chunks overwritten
//! 60       4    rows_overwritten    AtomicU32 — rows lost to ring wrap
//! 64       4    lease_offset        start of ReaderLease array
//! 68       4    lease_slots         slots per chunk
//! 72       8    instance_id         stable identity of this table incarnation
//! 80       8    writes_blocked      AtomicU64 — writes deferred by reader pins
//! 88       8    lease_failures      AtomicU64 — reader lease exhaustion
//! 96      32    _reserved_v5
//! ──────────────────────────────────────────────────────────
//! ```
//!
//! All multi-byte fields are little-endian.  The `byte_order` BOM
//! allows readers to detect endianness mismatch without guessing.
//!
//! MEMT is **single-writer**: exactly one writer owns each buffer (the
//! creator process; in-process writes are serialised by the caller). There
//! is no in-buffer write lock. Readers pin a chunk before borrowing row
//! bytes; recycling waits for those pins to drain before overwriting it.

use std::mem;
use std::sync::atomic::{AtomicI64, AtomicU32, AtomicU64};

// ── C-style layout structs ──────────────────────────────────────────

/// Magic number for MEMT (ring-buffer time-series table): bytes `M E M T` in little-endian.
pub const MAGIC_MEMT: u32 = 0x4D45_4D54;
pub(crate) const MAGIC: u32 = MAGIC_MEMT;

/// Header format version for MEMT.
///
/// v5: crash-recoverable per-process reader leases replace the unrecoverable
/// per-chunk pin counter.
pub(crate) const VERSION: u16 = 5;

/// Maximum number of processes that may concurrently pin the same chunk.
pub(crate) const READER_LEASE_SLOTS: usize = 16;

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

/// Fixed header at the start of every MemTable buffer (128 bytes).
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
    /// Size of this header in bytes (always 64).
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
    /// Padding to 8-align `creator_start_time` (was `write_lock` in v2).
    pub _pad0: u32,
    /// Process start time — for PID-recycling detection during discovery.
    /// Linux: clock ticks since boot (`/proc/<pid>/stat` field 22).
    /// macOS: microseconds since epoch (via `sysctl`).
    /// Other: 0 (falls back to PID-only liveness check).
    pub creator_start_time: u64,
    /// Ring chunks recycled with non-zero row data (hot-path counter).
    pub chunks_recycled: AtomicU32,
    /// Rows dropped because the ring buffer wrapped (hot-path counter).
    pub rows_overwritten: AtomicU32,
    /// Absolute offset of the per-chunk [`ReaderLease`] array.
    pub lease_offset: u32,
    /// Number of reader lease slots allocated for each chunk.
    pub lease_slots: u32,
    /// Stable, non-zero identity generated whenever a new table is initialized.
    ///
    /// Unlike `(name, chunk, generation)`, this distinguishes a newly-created
    /// table at the same path from its predecessor.
    pub instance_id: u64,
    /// Writes that could not recycle the next chunk because readers pinned it.
    pub writes_blocked: AtomicU64,
    /// Read attempts rejected because every process lease slot was occupied.
    pub reader_lease_failures: AtomicU64,
    pub _reserved_v5: [u64; 4],
}

/// Overwrite counters from a MEMT buffer header (for diagnostics / SQL plugins).
pub fn ring_overwrite_stats(buf: &[u8]) -> (u32, u32) {
    use std::sync::atomic::Ordering;
    let Some(h) = checked_header(buf) else {
        return (0, 0);
    };
    (
        h.chunks_recycled.load(Ordering::Relaxed),
        h.rows_overwritten.load(Ordering::Relaxed),
    )
}

/// Safely project a header from an arbitrary byte slice.
///
/// Internal callers use [`header`] only after construction/validation; public
/// byte-slice helpers must use this checked projection to avoid creating an
/// unaligned or out-of-bounds reference.
pub(crate) fn checked_header(buf: &[u8]) -> Option<&Header> {
    if buf.len() < mem::size_of::<Header>()
        || !(buf.as_ptr() as usize).is_multiple_of(mem::align_of::<Header>())
    {
        return None;
    }
    let h = unsafe { &*(buf.as_ptr() as *const Header) };
    (h.magic == MAGIC && h.version == VERSION && h.header_size as usize >= mem::size_of::<Header>())
        .then_some(h)
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
    pub _reserved: AtomicU32,
    /// Smallest value of the designated timestamp column in this chunk
    /// ([`TS_MIN_INIT`] when empty or no `Header::ts_col`). Maintained by
    /// the writer; readers must validate against `generation` snapshots.
    pub min_ts: AtomicI64,
    /// Largest timestamp in this chunk ([`TS_MAX_INIT`] when empty).
    pub max_ts: AtomicI64,
}

/// Crash-recoverable reader ownership for one process and one chunk.
///
/// `state` packs `owner_pid` in the high 32 bits and a local reference count
/// in the low 32 bits. A non-zero PID with zero refs is a short claim state
/// while `owner_start_time` is being published.
#[repr(C)]
pub(crate) struct ReaderLease {
    pub state: AtomicU64,
    pub owner_start_time: AtomicU64,
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
    assert!(mem::size_of::<Header>() == 128);
    assert!(mem::size_of::<ColumnDesc>() == 64);
    assert!(mem::size_of::<ChunkHeader>() == 40);
    assert!(mem::size_of::<ReaderLease>() == 16);
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
    debug_assert!(cs.is_multiple_of(8) && buf.len() >= cs + CHUNK_HEADER_SIZE);
    unsafe { &*(buf[cs..].as_ptr() as *const ChunkHeader) }
}

pub(crate) fn reader_lease(buf: &[u8], chunk: usize, slot: usize) -> &ReaderLease {
    let h = header(buf);
    debug_assert!(slot < h.lease_slots as usize);
    let index = chunk * h.lease_slots as usize + slot;
    let off = h.lease_offset as usize + index * mem::size_of::<ReaderLease>();
    debug_assert!(off.is_multiple_of(mem::align_of::<ReaderLease>()));
    debug_assert!(off + mem::size_of::<ReaderLease>() <= h.data_offset as usize);
    unsafe { &*(buf.as_ptr().add(off) as *const ReaderLease) }
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

pub(crate) fn checked_lease_offset(num_cols: usize) -> Option<usize> {
    mem::size_of::<ColumnDesc>()
        .checked_mul(num_cols)
        .and_then(|cols| mem::size_of::<Header>().checked_add(cols))
        .and_then(|bytes| bytes.checked_add(63))
        .map(|bytes| bytes & !63)
}

pub(crate) fn checked_data_offset(num_cols: usize, num_chunks: usize) -> Option<usize> {
    let lease_offset = checked_lease_offset(num_cols)?;
    mem::size_of::<ReaderLease>()
        .checked_mul(READER_LEASE_SLOTS)?
        .checked_mul(num_chunks)?
        .checked_add(lease_offset)?
        .checked_add(63)
        .map(|bytes| bytes & !63)
}

pub(crate) fn compute_data_offset(num_cols: usize, num_chunks: usize) -> usize {
    checked_data_offset(num_cols, num_chunks).expect("MEMT layout overflow")
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
        assert_eq!(mem::size_of::<Header>(), 128);
        assert_eq!(mem::size_of::<ColumnDesc>(), 64);
        assert_eq!(mem::size_of::<ChunkHeader>(), 40);
        assert_eq!(mem::size_of::<ReaderLease>(), 16);
    }

    #[test]
    fn byte_order_mark_sanity() {
        let bom = u16::from_ne_bytes(BYTE_ORDER_MARK);
        let expected_le = u16::from_le_bytes(BYTE_ORDER_MARK);
        assert_eq!(bom, expected_le);
    }

    #[test]
    fn public_stats_degrade_on_arbitrary_slices() {
        assert_eq!(ring_overwrite_stats(&[]), (0, 0));
        let bytes = [0u8; mem::size_of::<Header>() + 1];
        assert_eq!(ring_overwrite_stats(&bytes[1..]), (0, 0));
    }
}
