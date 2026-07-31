//! MEMH v5: self-describing pure key-value hash table with arena record log.
//!
//! ## Buffer layout
//!
//! ```text
//! offset     size   region
//! ─────────────────────────────────────────────────────────────
//!   0         128   MemhHeader  (metadata + writer recovery journal)
//! 128          32   MemhMeta    (slot geometry + hash seed)
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
//! +20   4B  generation u32 LE seqlock version (odd = writer active)
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
//! Every mutation acquires the cross-process `writer_pid` owner. Before a slot
//! changes, its four atomic words are copied into the header recovery journal.
//! If the owner dies, a reader or later writer verifies the process identity
//! and rolls an odd-generation slot back before taking ownership. Each slot is
//! accessed as four aligned `AtomicU64` words. The high half of word 2 is a
//! seqlock generation:
//! writers publish an odd generation before changing any word and an even
//! generation after the update. The first implementation uses `SeqCst` for
//! all slot accesses; readers copy atomic words and
//! accept the snapshot only when word 2 is unchanged and even.

use std::mem;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use crate::layout::{align64, BYTE_ORDER_MARK};

/// Magic number for MEMH: ASCII bytes `M E M H` in little-endian order.
pub const MAGIC_MEMH: u32 = 0x484D_454D;
/// Header format version for MEMH v5.
pub const VERSION_MEMH: u16 = 5;

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
/// `refcount` remains at byte 36, matching the MEMT header.
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
    pub arena_bump: AtomicU32,         // 32  bytes appended to arena so far
    pub refcount: AtomicU32,           // 36  layout-compatible with MEMT
    pub writer_pid: AtomicU32,         // 40  cross-process writer owner
    pub journal_state: AtomicU32,      // 44  0=empty, 1=valid
    pub writer_start_time: AtomicU64,  // 48
    pub creator_pid: u32,              // 56
    pub _pad_hot: u32,                 // 60
    pub creator_start_time: u64,       // 64
    pub journal_slot_off: AtomicU32,   // 72
    pub writer_claim: AtomicU32,       // 76  serialises owner publication
    pub journal_words: [AtomicU64; 4], // 80  pre-mutation slot image
    pub _reserved: [u64; 2],           // 112
}

/// Placed immediately after `MemhHeader` at offset 128.
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
    assert!(mem::size_of::<MemhHeader>() == 128);
    assert!(mem::size_of::<MemhMeta>() == 32);
};

// ── Offset helpers ────────────────────────────────────────

/// Absolute offset of `MemhMeta` in the buffer (always 128 in v5).
#[inline]
pub fn meta_offset() -> usize {
    mem::size_of::<MemhHeader>()
}

/// Absolute offset where the bucket array starts (64-aligned; always 192 in v5).
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

// ── Slot atomic access ────────────────────────────────────

const SLOT_WORDS: usize = SLOT_STRIDE / mem::size_of::<u64>();

#[inline(always)]
fn slot_word(buf: &[u8], slot_offset: usize, word: usize) -> &AtomicU64 {
    debug_assert!(word < SLOT_WORDS);
    let offset = slot_offset + word * mem::size_of::<u64>();
    debug_assert!(offset.is_multiple_of(mem::align_of::<AtomicU64>()));
    debug_assert!(offset + mem::size_of::<AtomicU64>() <= buf.len());
    unsafe { &*(buf.as_ptr().add(offset) as *const AtomicU64) }
}

#[inline(always)]
fn control_parts(control: u64) -> (u32, u32) {
    let bytes = control.to_le_bytes();
    (
        u32::from_le_bytes(bytes[..4].try_into().unwrap()),
        u32::from_le_bytes(bytes[4..].try_into().unwrap()),
    )
}

#[inline(always)]
fn pack_control(head_off: u32, generation: u32) -> u64 {
    let mut bytes = [0u8; 8];
    bytes[..4].copy_from_slice(&head_off.to_le_bytes());
    bytes[4..].copy_from_slice(&generation.to_le_bytes());
    u64::from_le_bytes(bytes)
}

#[inline(always)]
fn begin_slot_write(buf: &[u8], slot_offset: usize) -> (u32, u32) {
    let h = header(buf);
    for word in 0..SLOT_WORDS {
        h.journal_words[word].store(
            slot_word(buf, slot_offset, word).load(Ordering::SeqCst),
            Ordering::SeqCst,
        );
    }
    h.journal_slot_off
        .store(slot_offset as u32, Ordering::SeqCst);
    h.journal_state.store(1, Ordering::SeqCst);

    let control = slot_word(buf, slot_offset, 2);
    let (head_off, generation) = control_parts(control.load(Ordering::Relaxed));
    let odd = generation.wrapping_add(1) | 1;
    // SeqCst is intentionally conservative until the protocol has a model
    // proof with weaker orderings.
    control.store(pack_control(head_off, odd), Ordering::SeqCst);
    (head_off, odd)
}

#[inline(always)]
fn finish_slot_write(buf: &[u8], slot_offset: usize, head_off: u32, odd: u32) {
    slot_word(buf, slot_offset, 2).store(
        pack_control(head_off, odd.wrapping_add(1) & !1),
        Ordering::SeqCst,
    );
    header(buf).journal_state.store(0, Ordering::SeqCst);
}

/// Recover the single in-flight slot transaction after its writer died.
///
/// Odd generation means the slot update was not committed and the journal is
/// restored. Even generation means the slot commit completed and only stale
/// journal metadata needs clearing.
pub(crate) fn recover_slot_journal(buf: &[u8]) -> bool {
    let h = header(buf);
    if h.journal_state.load(Ordering::SeqCst) == 0 {
        return true;
    }
    let slot_offset = h.journal_slot_off.load(Ordering::SeqCst) as usize;
    let slots_end = h.data_offset as usize + h.num_buckets as usize * SLOT_STRIDE;
    if slot_offset < h.data_offset as usize
        || slot_offset + SLOT_STRIDE > slots_end
        || !slot_offset.is_multiple_of(mem::align_of::<AtomicU64>())
    {
        return false;
    }
    let (_, generation) = control_parts(slot_word(buf, slot_offset, 2).load(Ordering::SeqCst));
    if generation & 1 != 0 {
        for word in 0..SLOT_WORDS {
            slot_word(buf, slot_offset, word).store(
                h.journal_words[word].load(Ordering::SeqCst),
                Ordering::SeqCst,
            );
        }
    }
    h.journal_state.store(0, Ordering::SeqCst);
    true
}

pub(crate) struct MemhWriteGuard {
    header: *const MemhHeader,
}

impl Drop for MemhWriteGuard {
    fn drop(&mut self) {
        unsafe {
            let h = &*self.header;
            h.writer_start_time.store(0, Ordering::SeqCst);
            h.writer_pid.store(0, Ordering::SeqCst);
        }
    }
}

/// Acquire cross-process single-writer ownership, stealing it only from a
/// process instance proven dead. The new owner repairs any interrupted slot
/// transaction before returning.
pub(crate) fn acquire_writer(buf: &[u8]) -> Option<MemhWriteGuard> {
    let h = header(buf);
    let my_pid = std::process::id();
    let my_start = crate::raw::process_start_time(my_pid);

    for _ in 0..64 {
        let claimant = h.writer_claim.load(Ordering::SeqCst);
        if claimant != 0 {
            if crate::raw::process_exists(claimant) {
                return None;
            }
            let _ =
                h.writer_claim
                    .compare_exchange(claimant, 0, Ordering::SeqCst, Ordering::SeqCst);
            continue;
        }
        if h.writer_claim
            .compare_exchange(0, my_pid, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            continue;
        }

        let owner = h.writer_pid.load(Ordering::SeqCst);
        let can_claim = owner == 0
            || !crate::raw::process_instance_alive(
                owner,
                h.writer_start_time.load(Ordering::SeqCst),
            );
        if !can_claim {
            h.writer_claim.store(0, Ordering::SeqCst);
            return None;
        }
        h.writer_start_time.store(my_start, Ordering::SeqCst);
        h.writer_pid.store(my_pid, Ordering::SeqCst);
        h.writer_claim.store(0, Ordering::SeqCst);
        let guard = MemhWriteGuard {
            header: h as *const MemhHeader,
        };
        if recover_slot_journal(buf) {
            return Some(guard);
        }
        drop(guard);
        return None;
    }
    None
}

/// Atomically snapshot all slot fields.
///
/// Waits for an in-progress writer rather than fabricating `EMPTY`: treating a
/// busy slot as empty would break open-addressing probe chains and create false
/// misses. MEMH therefore requires its single writer not to die mid-slot update.
#[inline(always)]
pub fn read_slot(buf: &[u8], data_offset: usize, idx: usize) -> (u8, u8, u32, u64, u32, [u8; 8]) {
    let o = slot_off(data_offset, idx);
    debug_assert!(o + SLOT_STRIDE <= buf.len());
    let mut retries = 0usize;
    loop {
        let control_before = slot_word(buf, o, 2).load(Ordering::SeqCst);
        let (_, generation) = control_parts(control_before);
        if generation & 1 != 0 {
            if retries >= 64 && retries % 1024 == 64 {
                // A live owner keeps the slot busy. A dead owner is replaced
                // and its pre-mutation journal image restored.
                drop(acquire_writer(buf));
            }
            if retries < 64 {
                std::hint::spin_loop();
            } else {
                std::thread::yield_now();
            }
            retries = retries.saturating_add(1);
            continue;
        }

        // SeqCst is deliberately conservative here. It prevents payload loads
        // from moving past the second control-word validation. A benchmarked
        // weaker ordering must be model-checked before replacing it.
        let word0 = slot_word(buf, o, 0).load(Ordering::SeqCst);
        let hash = slot_word(buf, o, 1).load(Ordering::SeqCst);
        let value = slot_word(buf, o, 3).load(Ordering::SeqCst);
        let control_after = slot_word(buf, o, 2).load(Ordering::SeqCst);
        if control_before != control_after {
            std::hint::spin_loop();
            continue;
        }

        let word0 = word0.to_le_bytes();
        let (head_off, _) = control_parts(control_after);
        return (
            word0[0],
            word0[1],
            u32::from_le_bytes(word0[4..8].try_into().unwrap()),
            u64::from_le(hash),
            head_off,
            value.to_le_bytes(),
        );
    }
}

/// Read only the tag byte from a consistent slot snapshot.
#[inline]
pub fn read_slot_tag_acquire(buf: &[u8], data_offset: usize, idx: usize) -> u8 {
    read_slot(buf, data_offset, idx).0
}

// ── Slot write ────────────────────────────────────────────

/// Write all slot fields between odd/even seqlock generations.
///
/// Used for the **initial insert** of a new key (at an EMPTY or TOMBSTONE slot).
#[allow(clippy::too_many_arguments)]
pub(crate) fn commit_slot(
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
    let (_, odd) = begin_slot_write(buf, o);
    let mut word0 = [0u8; 8];
    word0[0] = tag;
    word0[1] = val_dtype;
    word0[4..8].copy_from_slice(&key_len.to_le_bytes());
    slot_word(buf, o, 0).store(u64::from_le_bytes(word0), Ordering::SeqCst);
    slot_word(buf, o, 1).store(hash.to_le(), Ordering::SeqCst);
    slot_word(buf, o, 3).store(u64::from_le_bytes(*val_bytes), Ordering::SeqCst);
    finish_slot_write(buf, o, head_off, odd);
}

/// Update only `head_off` and `tag` (used for **updates** and **deletes**).
///
/// `key_len`, `hash`, and `val_bytes` are left unchanged (same key, new record).
pub(crate) fn commit_slot_head(
    buf: &mut [u8],
    data_offset: usize,
    idx: usize,
    tag: u8,
    head_off: u32,
) {
    let o = slot_off(data_offset, idx);
    let (_, odd) = begin_slot_write(buf, o);
    let mut word0 = slot_word(buf, o, 0).load(Ordering::SeqCst).to_le_bytes();
    word0[0] = tag;
    slot_word(buf, o, 0).store(u64::from_le_bytes(word0), Ordering::SeqCst);
    finish_slot_write(buf, o, head_off, odd);
}

/// Update the inline scalar value in-place (**zero arena write** for scalar updates).
///
/// Does NOT change `head_off`; the existing PUT_INLINE arena record remains the
/// live head.
pub(crate) fn update_slot_inline_value(
    buf: &mut [u8],
    data_offset: usize,
    idx: usize,
    val_dtype: u8,
    val_bytes: &[u8; 8],
) {
    let o = slot_off(data_offset, idx);
    let (head_off, odd) = begin_slot_write(buf, o);
    let mut word0 = slot_word(buf, o, 0).load(Ordering::SeqCst).to_le_bytes();
    word0[0] = SLOT_INLINE;
    word0[1] = val_dtype;
    slot_word(buf, o, 0).store(u64::from_le_bytes(word0), Ordering::SeqCst);
    slot_word(buf, o, 3).store(u64::from_le_bytes(*val_bytes), Ordering::SeqCst);
    finish_slot_write(buf, o, head_off, odd);
}

/// Clear a slot to TOMBSTONE or EMPTY (used only during testing / compaction).
#[allow(dead_code)]
pub(crate) fn clear_slot(buf: &mut [u8], data_offset: usize, idx: usize, tombstone: bool) {
    let o = slot_off(data_offset, idx);
    let (_, odd) = begin_slot_write(buf, o);
    let tag = if tombstone {
        SLOT_TOMBSTONE
    } else {
        SLOT_EMPTY
    };
    slot_word(buf, o, 0).store(tag as u64, Ordering::SeqCst);
    slot_word(buf, o, 1).store(0, Ordering::SeqCst);
    slot_word(buf, o, 3).store(0, Ordering::SeqCst);
    finish_slot_write(buf, o, 0, odd);
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
    h.refcount.store(1, Ordering::Relaxed);
    h.writer_pid.store(0, Ordering::Relaxed);
    h.journal_state.store(0, Ordering::Relaxed);
    h.writer_start_time.store(0, Ordering::Relaxed);
    h.creator_pid = std::process::id();
    h._pad_hot = 0;
    h.creator_start_time = crate::raw::process_start_time(std::process::id());
    h.journal_slot_off.store(0, Ordering::Relaxed);
    h.writer_claim.store(0, Ordering::Relaxed);
    for word in &h.journal_words {
        word.store(0, Ordering::Relaxed);
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memh::table::{init_buf, MemhView, MemhWriter};
    use crate::schema::Value;

    #[test]
    fn dead_writer_odd_slot_is_rolled_back_from_journal() {
        let mut buf = vec![0u8; required_total_size(16, 4096)];
        init_buf(&mut buf, 16, 4096, 7).unwrap();
        MemhWriter::new(&mut buf)
            .unwrap()
            .insert("key", &Value::I64(42))
            .unwrap();

        let data_off = header(&buf).data_offset as usize;
        let idx = (0..16)
            .find(|&idx| {
                let tag = read_slot(&buf, data_off, idx).0;
                tag == SLOT_INLINE
            })
            .unwrap();
        let slot_offset = slot_off(data_off, idx);

        let dead_pid = 2_000_000_000u32;
        assert!(!crate::raw::process_exists(dead_pid));
        header(&buf).writer_start_time.store(123, Ordering::SeqCst);
        header(&buf).writer_pid.store(dead_pid, Ordering::SeqCst);
        let (_, _odd) = begin_slot_write(&buf, slot_offset);
        slot_word(&buf, slot_offset, 3).store(u64::MAX, Ordering::SeqCst);

        let view = MemhView::new(&buf).unwrap();
        assert_eq!(view.get("key"), Some(crate::memh::TypedValue::I64(42)));
        assert_eq!(header(&buf).writer_pid.load(Ordering::SeqCst), 0);
        assert_eq!(header(&buf).journal_state.load(Ordering::SeqCst), 0);
    }

    #[test]
    #[cfg(unix)]
    fn crashed_writer_process_is_recovered_by_reader() {
        let size = required_total_size(16, 4096);
        let shared = crate::test_mmap::TestSharedFile::new(size);
        let mut map = shared.map_mut();
        init_buf(&mut map, 16, 4096, 7).unwrap();
        MemhWriter::new(&mut map)
            .unwrap()
            .insert("key", &Value::I64(42))
            .unwrap();

        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("memh::layout::tests::crash_writer_child")
            .arg("--ignored")
            .env("PROBING_CRASH_MEMH_PATH", shared.path())
            .status()
            .unwrap();
        assert!(status.success());

        let view = MemhView::new(&map).unwrap();
        assert_eq!(view.get("key"), Some(crate::memh::TypedValue::I64(42)));
        assert_eq!(header(&map).writer_pid.load(Ordering::SeqCst), 0);
        assert_eq!(header(&map).journal_state.load(Ordering::SeqCst), 0);
    }

    #[test]
    #[ignore = "helper process for crashed_writer_process_is_recovered_by_reader"]
    #[cfg(unix)]
    fn crash_writer_child() {
        let Some(path) = std::env::var_os("PROBING_CRASH_MEMH_PATH") else {
            return;
        };
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .unwrap();
        let map = unsafe { memmap2::MmapMut::map_mut(&file).unwrap() };
        let _writer = acquire_writer(&map).unwrap();
        let data_off = header(&map).data_offset as usize;
        let idx = (0..16)
            .find(|&idx| read_slot(&map, data_off, idx).0 == SLOT_INLINE)
            .unwrap();
        let slot_offset = slot_off(data_off, idx);
        let (_, _odd) = begin_slot_write(&map, slot_offset);
        slot_word(&map, slot_offset, 3).store(u64::MAX, Ordering::SeqCst);
        std::process::exit(0);
    }
}
