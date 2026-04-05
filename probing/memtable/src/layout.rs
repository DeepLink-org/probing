//! Low-level layout: header, column descriptors, chunk headers, byte helpers.
//!
//! ## Header v2 binary layout (64 bytes, 1 cache line)
//!
//! ```text
//! offset  size  field               notes
//! ──────────────────────────────────────────────────────────
//!  0       4    magic               0x4D454D54 ("MEMT" in LE)
//!  4       2    version             2
//!  6       2    header_size         64 (validation only)
//!  8       2    byte_order          BOM: written as [0x01, 0x02]
//! 10       2    _pad0               0
//! 12       4    flags               feature bits (see FLAG_*)
//! 16       4    num_cols
//! 20       4    num_chunks
//! 24       4    chunk_size
//! 28       4    data_offset         (64-aligned)
//! ─── 32 byte boundary (cold/hot split) ─────────────────
//! 32       4    write_chunk         AtomicU32
//! 36       4    write_lock          AtomicU32
//! 40       4    refcount            AtomicU32
//! 44       4    creator_pid         PID of creating process
//! 48       8    creator_start_time  process start time (platform-specific)
//! 56       8    _reserved           0
//! ──────────────────────────────────────────────────────────
//! ```
//!
//! All multi-byte fields are little-endian.  The `byte_order` BOM
//! allows readers to detect endianness mismatch without guessing.

use std::mem;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

// ── C-style layout structs ──────────────────────────────────────────

pub(crate) const MAGIC: u32 = 0x4D45_4D54; // "MEMT"
pub(crate) const VERSION: u16 = 2;

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
/// `write_chunk`, `write_lock`, `refcount`.  Separated from the cold
/// zone to avoid false-sharing on different cache lines.
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
    pub _pad0: u16,
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
    /// Spinlock for writer serialization: 0 = unlocked, 1 = locked.
    pub write_lock: AtomicU32,
    /// Reference count for shared lifetime management.
    pub refcount: AtomicU32,
    /// PID of the process that created this table (for cross-process discovery).
    pub creator_pid: u32,
    /// Process start time — for PID-recycling detection.
    /// Linux: clock ticks since boot (`/proc/<pid>/stat` field 22).
    /// macOS: microseconds since epoch (via `sysctl`).
    /// Other: 0 (falls back to PID-only liveness check).
    pub creator_start_time: u64,
    pub _reserved: [u32; 2],
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

/// Per-chunk metadata, at the start of every chunk's byte region.
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
    assert!(mem::size_of::<ChunkHeader>() == 24);
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

/// Acquire the writer spinlock with exponential back-off.
///
/// First few failures use `spin_loop()` (pause instruction), then
/// escalate to `yield_now()` to avoid burning CPU under contention.
///
/// SAFETY NOTE: the buffer parameter is `&mut [u8]` (not `&[u8]`) so that
/// LLVM does **not** mark the pointer `readonly`. With `&[u8]` LLVM may
/// legally eliminate the atomic store inside `release_write_lock`, turning
/// the spin loop into an infinite loop in optimised (release) builds.
pub(crate) fn acquire_write_lock(buf: &mut [u8]) {
    let ptr = buf.as_mut_ptr() as *const Header;
    let lock = unsafe { &(*ptr).write_lock };
    let mut spins = 0u32;
    while lock
        .compare_exchange_weak(0, 1, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        if spins < 16 {
            for _ in 0..1 << spins.min(4) {
                std::hint::spin_loop();
            }
        } else {
            std::thread::yield_now();
        }
        spins += 1;
    }
}

/// Release the writer spinlock. See [`acquire_write_lock`] for why `&mut`.
pub(crate) fn release_write_lock(buf: &mut [u8]) {
    let ptr = buf.as_mut_ptr() as *const Header;
    unsafe { (*ptr).write_lock.store(0, Ordering::Release) };
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
        assert_eq!(mem::size_of::<ChunkHeader>(), 24);
    }

    #[test]
    fn byte_order_mark_sanity() {
        let bom = u16::from_ne_bytes(BYTE_ORDER_MARK);
        let expected_le = u16::from_le_bytes(BYTE_ORDER_MARK);
        assert_eq!(bom, expected_le);
    }
}
