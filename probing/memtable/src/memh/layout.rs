//! MEMH v3: self-describing pure key-value hash table with arena record log.
//!
//! ## Buffer layout
//!
//! ```text
//! offset     size   region
//! ─────────────────────────────────────────────────────────────
//!   0          64   MemhHeader  (cold zone + hot zone, 1 cache line)
//!  64          32   MemhMeta    (slot geometry + hash seed)
//!              --   64-byte alignment padding
//! 128        32×M   BucketSlot array (M = num_buckets, power-of-two)
//! 128+32×M    N    Arena (append-only ArenaRecord log)
//! ─────────────────────────────────────────────────────────────
//! ```
//!
//! ## BucketSlot layout (32 bytes)
//!
//! ```text
//!  +0   1B  tag        EMPTY=0 / TOMBSTONE=1 / OCCUPIED=2 / INLINE=3
//!  +1   1B  val_dtype  DType discriminant (valid only when tag=INLINE)
//!  +2   2B  _pad
//!  +4   4B  key_len    u32 LE, fast pre-filter
//!  +8   8B  hash       u64 LE, xxh3_64(key_bytes, seed)
//! +16   4B  head_off   u32 LE, absolute offset of latest ArenaRecord in buf
//! +20   4B  _gen       u32 LE (reserved, v1 = 0)
//! +24   8B  val_bytes  inline scalar value, LE zero-padded (valid only tag=INLINE)
//! ```
//!
//! - `EMPTY`    : slot never written; probe chain stops here.
//! - `TOMBSTONE`: key was deleted; head_off → TOMBSTONE record; probe chain continues.
//! - `OCCUPIED` : live key; head_off → PUT record (value in record payload).
//! - `INLINE`   : live key; head_off → PUT_INLINE record (value in val_bytes).
//!
//! ## ArenaRecord header layout (fixed 28 bytes, followed by variable payload)
//!
//! ```text
//!  +0   4B  record_len  u32 LE, total record bytes (header + payload, 4-byte aligned)
//!  +4   4B  slot_idx    u32 LE, which bucket owns this record (for iter liveness check)
//!  +8   8B  hash        u64 LE (redundant; enables compact/rebuild without re-hashing)
//! +16   4B  prev_off    u32 LE, absolute offset of previous version; NO_PREV=0xFFFF_FFFF
//! +20   2B  flags       u16 LE: PUT=1 / TOMBSTONE=2 / PUT_INLINE=3
//! +22   2B  key_len     u16 LE
//! +24   4B  val_dtype   u32 LE (0xFFFF_FFFF for PUT_INLINE and TOMBSTONE)
//! --- payload at +28 ---
//!          [key_bytes][val_payload]   (PUT_INLINE and TOMBSTONE have no val_payload)
//! ```
//!
//! ## Concurrency model
//!
//! `MemhWriter` holds `&mut [u8]`, which guarantees exclusive write access at
//! compile time — no runtime spinlock is needed.  After writing payload, a
//! `fence(Release)` + `write_volatile(tag)` makes the slot visible to lock-free
//! readers, whose `read_volatile(tag)` + `fence(Acquire)` pairs with it.

use std::mem;
use std::sync::atomic::{self, AtomicU32, Ordering};

use crate::layout::{align64, BYTE_ORDER_MARK};

/// Magic number for MEMH: ASCII bytes `M E M H` in little-endian order.
pub const MAGIC_MEMH: u32 = 0x484D_454D;
/// Header format version for MEMH v3.
pub const VERSION_MEMH: u16 = 3;

/// Byte stride of one bucket slot.
pub const SLOT_STRIDE: usize = 32;

/// Slot tag: bucket has never been written; terminates probe chains.
pub const SLOT_EMPTY: u8 = 0;
/// Slot tag: key was deleted; probe chains must continue past this slot.
pub const SLOT_TOMBSTONE: u8 = 1;
/// Slot tag: live key; value stored in the arena record at `head_off`.
pub const SLOT_OCCUPIED: u8 = 2;
/// Slot tag: live key; scalar value stored inline in `val_bytes`; arena record
/// at `head_off` holds only the key (flags=PUT_INLINE).
pub const SLOT_INLINE: u8 = 3;

// ── Fixed header (64 bytes = 1 cache line) ───────────────

/// Fixed header placed at offset 0.
///
/// **Cold zone** (bytes 0–31): immutable after init.
/// **Hot zone** (bytes 32–63): atomically mutated at runtime; offsets match
/// the MEMT `Header` so shared refcount/write-lock helpers are
/// layout-compatible.
#[repr(C)]
pub struct MemhHeader {
    // cold zone
    pub magic: u32,               //  0
    pub version: u16,             //  4
    pub header_size: u16,         //  6
    pub byte_order: u16,          //  8  BOM = 0x0102
    pub _pad0: u16,               // 10
    pub flags: u32,               // 12
    pub num_buckets: u32,         // 16  must be power-of-two
    pub data_offset: u32,         // 20  start of bucket array (64-aligned)
    pub _reserved_cold: [u32; 2], // 24
    // hot zone
    pub arena_bump: AtomicU32,   // 32  bytes appended to arena so far
    pub write_lock: AtomicU32,   // 36  spinlock: 0 = free
    pub refcount: AtomicU32,     // 40
    pub creator_pid: u32,        // 44
    pub creator_start_time: u64, // 48
    pub _reserved: [u32; 2],     // 56
}

/// Placed immediately after `MemhHeader` at offset 64.
#[repr(C)]
pub struct MemhMeta {
    pub slot_stride: u32, //  0  always SLOT_STRIDE (for validation)
    pub _pad: u32,        //  4
    pub arena_start: u32, //  8  absolute start of the arena in the buffer
    pub arena_cap: u32,   // 12  arena capacity in bytes
    pub hash_seed: u64,   // 16  xxh3_64 seed
    pub _reserved: u64,   // 24
}

const _: () = {
    assert!(mem::size_of::<MemhHeader>() == 64);
    assert!(mem::size_of::<MemhMeta>() == 32);
};

// ── Offset helpers ────────────────────────────────────────

/// Absolute offset of `MemhMeta` in the buffer (always 64 in v3).
#[inline]
pub fn meta_offset() -> usize {
    mem::size_of::<MemhHeader>()
}

/// Absolute offset where the bucket array starts (64-aligned; always 128 in v3).
pub fn compute_data_offset() -> usize {
    align64(meta_offset() + mem::size_of::<MemhMeta>())
}

/// Absolute start of the arena.
pub fn arena_start_abs(num_buckets: u32, data_off: usize) -> usize {
    data_off + num_buckets as usize * SLOT_STRIDE
}

/// Minimum buffer size for the given parameters.
pub fn required_total_size(num_buckets: u32, arena_cap: usize) -> usize {
    arena_start_abs(num_buckets, compute_data_offset()) + arena_cap
}

// ── Struct accessors ──────────────────────────────────────

#[inline]
pub fn header(buf: &[u8]) -> &MemhHeader {
    debug_assert!(buf.len() >= mem::size_of::<MemhHeader>());
    unsafe { &*(buf.as_ptr() as *const MemhHeader) }
}

#[inline]
pub fn header_mut(buf: &mut [u8]) -> &mut MemhHeader {
    debug_assert!(buf.len() >= mem::size_of::<MemhHeader>());
    unsafe { &mut *(buf.as_mut_ptr() as *mut MemhHeader) }
}

#[inline]
pub fn meta(buf: &[u8]) -> &MemhMeta {
    let off = meta_offset();
    debug_assert!(buf.len() >= off + mem::size_of::<MemhMeta>());
    unsafe { &*(buf.as_ptr().add(off) as *const MemhMeta) }
}

#[inline]
pub fn meta_mut(buf: &mut [u8]) -> &mut MemhMeta {
    let off = meta_offset();
    debug_assert!(buf.len() >= off + mem::size_of::<MemhMeta>());
    unsafe { &mut *(buf.as_mut_ptr().add(off) as *mut MemhMeta) }
}

/// Byte offset of slot `idx` within `buf`.
#[inline]
pub fn slot_off(data_offset: usize, idx: usize) -> usize {
    data_offset + idx * SLOT_STRIDE
}

// ── Slot read ─────────────────────────────────────────────

/// Read all slot fields with a single volatile+Acquire.
///
/// - One `read_volatile(tag)` + one `fence(Acquire)` establishes the happens-before
///   edge with the writer's `fence(Release)` + `write_volatile(tag)`.
/// - All remaining fields are read from the same 32-byte cache line via plain
///   (non-volatile) loads, which is safe after the Acquire fence.
#[inline(always)]
pub fn read_slot(buf: &[u8], data_offset: usize, idx: usize) -> (u8, u8, u32, u64, u32, [u8; 8]) {
    let o = slot_off(data_offset, idx);
    debug_assert!(o + SLOT_STRIDE <= buf.len());
    unsafe {
        let p = buf.as_ptr().add(o);
        let tag = std::ptr::read_volatile(p);
        atomic::fence(Ordering::Acquire);
        let val_dtype = *p.add(1);
        let key_len = u32::from_le_bytes(*p.add(4).cast::<[u8; 4]>());
        let hash = u64::from_le_bytes(*p.add(8).cast::<[u8; 8]>());
        let head_off = u32::from_le_bytes(*p.add(16).cast::<[u8; 4]>());
        let mut val_bytes = [0u8; 8];
        std::ptr::copy_nonoverlapping(p.add(24), val_bytes.as_mut_ptr(), 8);
        (tag, val_dtype, key_len, hash, head_off, val_bytes)
    }
}

/// Read only the tag byte (used only in `validate_memh` where we skip full slot reads).
#[inline]
pub fn read_slot_tag_acquire(buf: &[u8], data_offset: usize, idx: usize) -> u8 {
    let o = slot_off(data_offset, idx);
    let tag = unsafe { std::ptr::read_volatile(buf.as_ptr().add(o)) };
    atomic::fence(Ordering::Acquire);
    tag
}

// ── Slot write ────────────────────────────────────────────

/// Write all slot fields, fence(Release), then atomically publish `tag`.
///
/// Used for the **initial insert** of a new key (at an EMPTY or TOMBSTONE slot).
pub fn commit_slot(
    buf: &mut [u8],
    data_offset: usize,
    idx: usize,
    tag: u8,
    val_dtype: u8,
    key_len: u32,
    hash: u64,
    head_off: u32,
    val_bytes: &[u8; 8],
) {
    let o = slot_off(data_offset, idx);
    buf[o + 1] = val_dtype;
    buf[o + 2..o + 4].fill(0); // _pad
    buf[o + 4..o + 8].copy_from_slice(&key_len.to_le_bytes());
    buf[o + 8..o + 16].copy_from_slice(&hash.to_le_bytes());
    buf[o + 16..o + 20].copy_from_slice(&head_off.to_le_bytes());
    buf[o + 20..o + 24].fill(0); // _gen
    buf[o + 24..o + 32].copy_from_slice(val_bytes);
    atomic::fence(Ordering::Release);
    unsafe { std::ptr::write_volatile(buf.as_mut_ptr().add(o), tag) };
}

/// Update only `head_off` and `tag` (used for **updates** and **deletes**).
///
/// `key_len`, `hash`, and `val_bytes` are left unchanged (same key, new record).
pub fn commit_slot_head(buf: &mut [u8], data_offset: usize, idx: usize, tag: u8, head_off: u32) {
    let o = slot_off(data_offset, idx);
    buf[o + 16..o + 20].copy_from_slice(&head_off.to_le_bytes());
    atomic::fence(Ordering::Release);
    unsafe { std::ptr::write_volatile(buf.as_mut_ptr().add(o), tag) };
}

/// Update the inline scalar value in-place (**zero arena write** for scalar updates).
///
/// Does NOT change `head_off`; the existing PUT_INLINE arena record remains the
/// live head.  Writes a "dummy" volatile store of SLOT_INLINE to the tag byte to
/// create a happens-before edge for lock-free readers.
pub fn update_slot_inline_value(
    buf: &mut [u8],
    data_offset: usize,
    idx: usize,
    val_dtype: u8,
    val_bytes: &[u8; 8],
) {
    let o = slot_off(data_offset, idx);
    buf[o + 24..o + 32].copy_from_slice(val_bytes);
    buf[o + 1] = val_dtype;
    atomic::fence(Ordering::Release);
    // Dummy volatile write of the same tag value to create a Release/Acquire
    // pairing point for lock-free readers that re-read the slot tag.
    unsafe { std::ptr::write_volatile(buf.as_mut_ptr().add(o), SLOT_INLINE) };
}

/// Clear a slot to TOMBSTONE or EMPTY (used only during testing / compaction).
pub fn clear_slot(buf: &mut [u8], data_offset: usize, idx: usize, tombstone: bool) {
    let o = slot_off(data_offset, idx);
    buf[o + 1..o + SLOT_STRIDE].fill(0);
    let tag = if tombstone {
        SLOT_TOMBSTONE
    } else {
        SLOT_EMPTY
    };
    atomic::fence(Ordering::Release);
    unsafe { std::ptr::write_volatile(buf.as_mut_ptr().add(o), tag) };
}

// ── Initialisation helpers ────────────────────────────────

pub fn init_header(h: &mut MemhHeader, num_buckets: u32, data_off: u32) {
    h.magic = MAGIC_MEMH;
    h.version = VERSION_MEMH;
    h.header_size = mem::size_of::<MemhHeader>() as u16;
    h.byte_order = u16::from_ne_bytes(BYTE_ORDER_MARK);
    h._pad0 = 0;
    h.flags = 0;
    h.num_buckets = num_buckets;
    h.data_offset = data_off;
    h._reserved_cold = [0; 2];
    h.arena_bump.store(0, Ordering::Relaxed);
    h.write_lock.store(0, Ordering::Relaxed);
    h.refcount.store(1, Ordering::Relaxed);
    h.creator_pid = std::process::id();
    h.creator_start_time = crate::raw::process_start_time(std::process::id());
    h._reserved = [0; 2];
}

pub fn init_meta_fields(buf: &mut [u8], arena_start: u32, arena_cap: u32, hash_seed: u64) {
    let m = meta_mut(buf);
    m.slot_stride = SLOT_STRIDE as u32;
    m._pad = 0;
    m.arena_start = arena_start;
    m.arena_cap = arena_cap;
    m.hash_seed = hash_seed;
    m._reserved = 0;
}
