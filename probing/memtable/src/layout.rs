//! Low-level layout: header, column descriptors, chunk headers, byte helpers.

use std::mem;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

// ── C-style layout structs ──────────────────────────────────────────

pub const MAGIC: u32 = 0x4D45_4D54; // "MEMT"
pub const VERSION: u32 = 1;

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
    pub(crate) fn from_u32(v: u32) -> Self {
        match v {
            0 => Self::Empty,
            1 => Self::Writing,
            2 => Self::Sealed,
            _ => Self::Empty,
        }
    }
}

pub(crate) const CHUNK_HEADER_SIZE: usize = mem::size_of::<ChunkHeader>();

const _: () = {
    assert!(mem::size_of::<Header>() == 36);
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

/// Acquire the writer spinlock.
///
/// SAFETY NOTE: the buffer parameter is `&mut [u8]` (not `&[u8]`) so that
/// LLVM does **not** mark the pointer `readonly`. With `&[u8]` LLVM may
/// legally eliminate the atomic store inside `release_write_lock`, turning
/// the spin loop into an infinite loop in optimised (release) builds.
pub(crate) fn acquire_write_lock(buf: &mut [u8]) {
    let ptr = buf.as_mut_ptr() as *const Header;
    let lock = unsafe { &(*ptr).write_lock };
    while lock
        .compare_exchange_weak(0, 1, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        std::hint::spin_loop();
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
        assert_eq!(mem::size_of::<Header>(), 36);
        assert_eq!(mem::size_of::<ColumnDesc>(), 64);
        assert_eq!(mem::size_of::<ChunkHeader>(), 24);
    }
}
