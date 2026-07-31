//! `MemhView` (reader) and `MemhWriter` (single-writer) for MEMH v5.
//!
//! See `layout.rs` for the full buffer and slot layouts, and `codec.rs` for
//! arena record encoding / decoding details.

use std::sync::atomic::Ordering;

use crate::schema::Value;
use crate::sync::lock_mutex;

use super::codec::{
    encode_inline_bytes, encode_put_inline_record, encode_put_record, encode_tombstone_record,
    hash_key, is_scalar_dtype, read_record_header, record_key, record_value, slot_inline_value,
    TypedValue, ARENA_HDR_SIZE, NO_PREV,
};
use super::layout::{
    acquire_writer, commit_slot, commit_slot_head, compute_data_offset, header, header_mut,
    init_header, init_meta_fields, meta, read_slot, read_slot_tag_acquire,
    update_slot_inline_value, MemhHeader, MAGIC_MEMH, SLOT_EMPTY, SLOT_INLINE, SLOT_OCCUPIED,
    SLOT_TOMBSTONE, VERSION_MEMH,
};

// ── Errors ───────────────────────────────────────────────

#[derive(Debug)]
pub enum MemhInitError {
    BufferTooSmall { need: usize, got: usize },
    MisalignedBuffer { required: usize },
    BucketsNotPowerOfTwo(u32),
    BucketsZero,
    ArenaTooSmall,
    ArenaTooLarge,
}

impl std::error::Error for MemhInitError {}

impl std::fmt::Display for MemhInitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MemhInitError::BufferTooSmall { need, got } => {
                write!(f, "buffer too small: need {need}, got {got}")
            }
            MemhInitError::MisalignedBuffer { required } => {
                write!(f, "buffer address must be aligned to {required} bytes")
            }
            MemhInitError::BucketsNotPowerOfTwo(n) => {
                write!(f, "num_buckets {n} is not a power of two")
            }
            MemhInitError::BucketsZero => write!(f, "num_buckets must be > 0"),
            MemhInitError::ArenaTooSmall => write!(f, "arena_cap must be > 0"),
            MemhInitError::ArenaTooLarge => write!(f, "arena_cap exceeds the u32 MEMH format"),
        }
    }
}

#[derive(Debug)]
pub enum MemhValidateError {
    TooShort,
    MisalignedBuffer { required: usize },
    WrongMagic(u32),
    UnsupportedVersion(u16),
    InvalidLayout(&'static str),
    CorruptEntry { bucket: usize },
}

impl std::fmt::Display for MemhValidateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MemhValidateError::TooShort => write!(f, "buffer too short for a MEMH table"),
            MemhValidateError::MisalignedBuffer { required } => {
                write!(f, "buffer address must be aligned to {required} bytes")
            }
            MemhValidateError::WrongMagic(m) => write!(f, "wrong magic 0x{m:08X}"),
            MemhValidateError::UnsupportedVersion(v) => write!(f, "unsupported MEMH version {v}"),
            MemhValidateError::InvalidLayout(message) => {
                write!(f, "invalid MEMH layout: {message}")
            }
            MemhValidateError::CorruptEntry { bucket } => {
                write!(f, "corrupt arena entry at bucket {bucket}")
            }
        }
    }
}

impl std::error::Error for MemhValidateError {}

#[derive(Debug, PartialEq, Eq)]
pub enum InsertResult {
    Inserted,
    Updated,
}

#[derive(Debug)]
pub enum InsertError {
    TableFull,
    ArenaFull,
    InvalidValue,
    WriterBusy,
}

impl std::fmt::Display for InsertError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InsertError::TableFull => write!(f, "hash table is full"),
            InsertError::ArenaFull => write!(f, "arena is full"),
            InsertError::InvalidValue => write!(f, "value cannot be stored inline"),
            InsertError::WriterBusy => write!(f, "another process owns the MEMH writer"),
        }
    }
}

// ── Buffer initialisation ─────────────────────────────────

/// Initialise `buf` as a fresh MEMH v5 table.
///
/// - `num_buckets`: must be a power of two
/// - `arena_cap`: arena capacity in bytes (> 0)
/// - `hash_seed`: xxh3-64 seed; pass `0` for the default
pub fn init_buf(
    buf: &mut [u8],
    num_buckets: u32,
    arena_cap: usize,
    hash_seed: u64,
) -> Result<(), MemhInitError> {
    let required_alignment = std::mem::align_of::<MemhHeader>();
    if !(buf.as_ptr() as usize).is_multiple_of(required_alignment) {
        return Err(MemhInitError::MisalignedBuffer {
            required: required_alignment,
        });
    }
    if num_buckets == 0 {
        return Err(MemhInitError::BucketsZero);
    }
    if num_buckets & (num_buckets - 1) != 0 {
        return Err(MemhInitError::BucketsNotPowerOfTwo(num_buckets));
    }
    if arena_cap == 0 {
        return Err(MemhInitError::ArenaTooSmall);
    }
    if arena_cap > u32::MAX as usize {
        return Err(MemhInitError::ArenaTooLarge);
    }
    let data_off = compute_data_offset();
    let arena_start = (num_buckets as usize)
        .checked_mul(super::layout::SLOT_STRIDE)
        .and_then(|slots| data_off.checked_add(slots))
        .ok_or(MemhInitError::ArenaTooLarge)?;
    let need = arena_start
        .checked_add(arena_cap)
        .ok_or(MemhInitError::ArenaTooLarge)?;
    if buf.len() < need {
        return Err(MemhInitError::BufferTooSmall {
            need,
            got: buf.len(),
        });
    }
    buf[..need].fill(0);
    init_header(header_mut(buf), num_buckets, data_off as u32);
    init_meta_fields(buf, arena_start as u32, arena_cap as u32, hash_seed);
    Ok(())
}

/// Validate that `buf` contains a well-formed MEMH v5 table header.
pub fn validate_memh(buf: &[u8]) -> Result<(), MemhValidateError> {
    use super::layout::{MemhHeader, MemhMeta};
    use std::mem::size_of;

    if buf.len() < size_of::<MemhHeader>() + size_of::<MemhMeta>() {
        return Err(MemhValidateError::TooShort);
    }
    let required_alignment = std::mem::align_of::<MemhHeader>();
    if !(buf.as_ptr() as usize).is_multiple_of(required_alignment) {
        return Err(MemhValidateError::MisalignedBuffer {
            required: required_alignment,
        });
    }
    let h = header(buf);
    if h.magic != MAGIC_MEMH {
        return Err(MemhValidateError::WrongMagic(h.magic));
    }
    if h.version != VERSION_MEMH {
        return Err(MemhValidateError::UnsupportedVersion(h.version));
    }
    if h.header_size as usize != size_of::<MemhHeader>() {
        return Err(MemhValidateError::InvalidLayout("invalid header_size"));
    }
    if h.byte_order != u16::from_ne_bytes(crate::layout::BYTE_ORDER_MARK) {
        return Err(MemhValidateError::InvalidLayout("byte order mismatch"));
    }
    if h.num_buckets == 0 || !h.num_buckets.is_power_of_two() {
        return Err(MemhValidateError::InvalidLayout(
            "num_buckets must be a non-zero power of two",
        ));
    }

    let m = meta(buf);
    let data_off = h.data_offset as usize;
    let num_buckets = h.num_buckets as usize;
    let arena_start = m.arena_start as usize;
    let arena_cap = m.arena_cap as usize;
    let arena_bump = h.arena_bump.load(Ordering::Acquire) as usize;
    if data_off != compute_data_offset() {
        return Err(MemhValidateError::InvalidLayout("invalid data_offset"));
    }
    if m.slot_stride as usize != super::layout::SLOT_STRIDE {
        return Err(MemhValidateError::InvalidLayout("invalid slot_stride"));
    }
    let expected_arena_start = num_buckets
        .checked_mul(super::layout::SLOT_STRIDE)
        .and_then(|slots| data_off.checked_add(slots))
        .ok_or(MemhValidateError::InvalidLayout("bucket layout overflow"))?;
    if arena_start != expected_arena_start {
        return Err(MemhValidateError::InvalidLayout("invalid arena_start"));
    }
    let arena_end = arena_start
        .checked_add(arena_cap)
        .ok_or(MemhValidateError::InvalidLayout("arena layout overflow"))?;
    if arena_end > buf.len() {
        return Err(MemhValidateError::InvalidLayout("arena exceeds buffer"));
    }
    if arena_bump > arena_cap {
        return Err(MemhValidateError::InvalidLayout(
            "arena bump exceeds capacity",
        ));
    }
    for idx in 0..num_buckets {
        // Single slot read covers both tag and head_off.
        let (tag, _, _, _, head_off, _) = read_slot(buf, data_off, idx);
        if tag > SLOT_INLINE {
            return Err(MemhValidateError::CorruptEntry { bucket: idx });
        }
        if tag == SLOT_EMPTY {
            continue;
        }

        // TOMBSTONE slots may have head_off = u32::MAX when the arena was full
        // at removal time — this is the "no arena record" sentinel and is valid.
        if tag == SLOT_TOMBSTONE && head_off == u32::MAX {
            continue;
        }

        let ho = head_off as usize;
        let Some(header_end) = ho.checked_add(ARENA_HDR_SIZE) else {
            return Err(MemhValidateError::CorruptEntry { bucket: idx });
        };
        // Acquire a fresh publication bound after the slot snapshot. A writer
        // may append and publish this slot after the initial geometry check.
        let live_arena_end = arena_start + h.arena_bump.load(Ordering::Acquire) as usize;
        if ho < arena_start || header_end > live_arena_end {
            return Err(MemhValidateError::CorruptEntry { bucket: idx });
        }
        let Some(record) = read_record_header(&buf[ho..live_arena_end]) else {
            return Err(MemhValidateError::CorruptEntry { bucket: idx });
        };
        let Some(record_end) = ho.checked_add(record.record_len as usize) else {
            return Err(MemhValidateError::CorruptEntry { bucket: idx });
        };
        if record.record_len < ARENA_HDR_SIZE as u32
            || record_end > live_arena_end
            || record.slot_idx as usize != idx
            || record_key(&record, &buf[ho..record_end]).is_none()
        {
            return Err(MemhValidateError::CorruptEntry { bucket: idx });
        }
    }
    Ok(())
}

// ── Read view ─────────────────────────────────────────────

/// Immutable view over an MEMH buffer.  Multiple concurrent readers are safe.
pub struct MemhView<'a> {
    buf: &'a [u8],
    data_off: usize,
    num_buckets: usize,
    mask: usize,
    arena_start: usize,
    hash_seed: u64,
}

impl<'a> MemhView<'a> {
    pub fn new(buf: &'a [u8]) -> Result<Self, MemhValidateError> {
        validate_memh(buf)?;
        let h = header(buf);
        let m = meta(buf);
        Ok(Self {
            buf,
            data_off: h.data_offset as usize,
            num_buckets: h.num_buckets as usize,
            mask: h.num_buckets as usize - 1,
            arena_start: m.arena_start as usize,
            hash_seed: m.hash_seed,
        })
    }

    /// Look up `key` and return its current value, or `None` if absent.
    pub fn get(&self, key: &str) -> Option<TypedValue<'_>> {
        let kh = hash_key(self.hash_seed, key);
        let probe_start = kh as usize & self.mask;

        for probe in 0..self.num_buckets {
            let idx = (probe_start + probe) & self.mask;
            // One seqlock snapshot covers tag and all slot fields.
            let (tag, val_dtype, key_len, slot_hash, head_off, val_bytes) =
                read_slot(self.buf, self.data_off, idx);
            match tag {
                SLOT_EMPTY => return None,
                SLOT_TOMBSTONE => continue,
                SLOT_OCCUPIED | SLOT_INLINE => {
                    if slot_hash != kh {
                        continue;
                    }
                    if key_len as usize != key.len() {
                        continue;
                    }
                    let ho = head_off as usize;
                    let bump = header(self.buf).arena_bump.load(Ordering::Acquire) as usize;
                    let live_end = self.arena_start.checked_add(bump)?;
                    if ho < self.arena_start || ho.checked_add(ARENA_HDR_SIZE)? > live_end {
                        return None;
                    }
                    let record_data = self.buf.get(ho..live_end)?;
                    let hdr = read_record_header(record_data)?;
                    let record_end = ho.checked_add(hdr.record_len as usize)?;
                    if hdr.record_len < ARENA_HDR_SIZE as u32 || record_end > live_end {
                        return None;
                    }
                    let record_data = self.buf.get(ho..record_end)?;
                    let rkey = record_key(&hdr, record_data)?;
                    if rkey != key {
                        continue;
                    }
                    if tag == SLOT_INLINE {
                        return slot_inline_value(val_dtype, &val_bytes);
                    }
                    return record_value(&hdr, record_data);
                }
                _ => continue,
            }
        }
        None
    }

    /// Iterate over one stable snapshot of each live slot.
    ///
    /// This is weakly consistent across slots, not a whole-table point-in-time
    /// snapshot, but every yielded key/value pair is internally consistent.
    pub fn iter(&self) -> impl Iterator<Item = (&str, TypedValue<'_>)> + '_ {
        let arena_start = self.arena_start;
        let buf = self.buf;
        let data_off = self.data_off;
        let num_buckets = self.num_buckets;

        let mut idx = 0usize;
        std::iter::from_fn(move || {
            while idx < num_buckets {
                let slot_idx = idx;
                idx += 1;
                let (tag, val_dtype, _, _, head_off, val_bytes) =
                    read_slot(buf, data_off, slot_idx);
                if tag != SLOT_OCCUPIED && tag != SLOT_INLINE {
                    continue;
                }
                let abs_pos = head_off as usize;
                let bump = header(buf).arena_bump.load(Ordering::Acquire) as usize;
                let live_end = arena_start.checked_add(bump)?;
                if abs_pos < arena_start || abs_pos.checked_add(ARENA_HDR_SIZE)? > live_end {
                    continue;
                }
                let record_data = buf.get(abs_pos..live_end)?;
                let hdr = read_record_header(record_data)?;
                let record_len = hdr.record_len as usize;
                let record_end = abs_pos.checked_add(record_len)?;
                if record_len < ARENA_HDR_SIZE
                    || record_end > live_end
                    || hdr.slot_idx as usize != slot_idx
                {
                    continue;
                }
                let record_data = buf.get(abs_pos..record_end)?;
                let key = record_key(&hdr, record_data)?;
                match (tag, hdr.flags) {
                    (SLOT_OCCUPIED, super::codec::FLAG_PUT) => {
                        let value = record_value(&hdr, record_data)?;
                        return Some((key, value));
                    }
                    (SLOT_INLINE, super::codec::FLAG_PUT_INLINE) => {
                        let value = slot_inline_value(val_dtype, &val_bytes)?;
                        return Some((key, value));
                    }
                    _ => continue,
                }
            }
            None
        })
    }

    /// Number of live entries.
    pub fn len(&self) -> usize {
        (0..self.num_buckets)
            .filter(|&i| {
                let t = read_slot_tag_acquire(self.buf, self.data_off, i);
                t == SLOT_OCCUPIED || t == SLOT_INLINE
            })
            .count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

// ── Write view ────────────────────────────────────────────

/// Single-writer view over an MEMH buffer (enforced by Rust's `&mut` rules).
pub struct MemhWriter<'a> {
    buf: &'a mut [u8],
    data_off: usize,
    num_buckets: usize,
    mask: usize,
    arena_start: usize,
    arena_cap: usize,
    hash_seed: u64,
    scratch: Vec<u8>,
}

impl<'a> MemhWriter<'a> {
    pub fn new(buf: &'a mut [u8]) -> Result<Self, MemhValidateError> {
        validate_memh(buf)?;
        Ok(unsafe { Self::new_unchecked(buf) })
    }

    /// Create a writer without running `validate_memh`.
    ///
    /// # Safety
    /// `buf` must contain a valid, fully-initialised MEMH v5 table.
    /// Callers that have already validated the buffer (e.g. [`SharedMemhWriter`])
    /// may use this to avoid the O(num_buckets) validation cost on every lock
    /// acquisition.
    pub(crate) unsafe fn new_unchecked(buf: &'a mut [u8]) -> Self {
        let (data_off, num_buckets, arena_start, arena_cap, hash_seed) = {
            let h = header(buf);
            let m = meta(buf);
            (
                h.data_offset as usize,
                h.num_buckets as usize,
                m.arena_start as usize,
                m.arena_cap as usize,
                m.hash_seed,
            )
        };
        Self {
            buf,
            data_off,
            num_buckets,
            mask: num_buckets - 1,
            arena_start,
            arena_cap,
            hash_seed,
            scratch: Vec::with_capacity(64),
        }
    }

    /// Insert or update a key-value pair.
    ///
    /// **Scalar values** (`U8`/`U32`/`I32`/`U64`/`I64`/`F32`/`F64`) are stored
    /// inline in the slot.  Updating an existing scalar key with another scalar
    /// writes **zero** arena bytes.
    ///
    /// **Variable-length values** (`Str`/`Bytes`) are always appended to the arena.
    ///
    /// # Concurrency
    ///
    /// `&mut [u8]` serialises one mapping; the header owner additionally
    /// serialises writers from independent shared mappings and recovers a
    /// transaction left by a dead writer process.
    pub fn insert(&mut self, key: &str, val: &Value<'_>) -> Result<InsertResult, InsertError> {
        let _writer = acquire_writer(self.buf).ok_or(InsertError::WriterBusy)?;
        self.insert_locked(key, val)
    }

    fn insert_locked(&mut self, key: &str, val: &Value<'_>) -> Result<InsertResult, InsertError> {
        let kh = hash_key(self.hash_seed, key);
        let is_scalar = is_scalar_dtype(val.dtype());
        let probe_start = kh as usize & self.mask;

        let bump = header(self.buf).arena_bump.load(Ordering::Relaxed) as usize;
        let mut first_tomb: Option<usize> = None;

        for probe in 0..self.num_buckets {
            let idx = (probe_start + probe) & self.mask;
            // One seqlock snapshot covers tag and all fields.
            let (tag, _, key_len, slot_hash, head_off, _) = read_slot(self.buf, self.data_off, idx);

            match tag {
                SLOT_EMPTY => {
                    let target = first_tomb.unwrap_or(idx);
                    return self
                        .write_new_entry(target, key, val, kh, bump, is_scalar)
                        .map(|_| InsertResult::Inserted);
                }
                SLOT_TOMBSTONE => {
                    if first_tomb.is_none() {
                        first_tomb = Some(idx);
                    }
                }
                SLOT_OCCUPIED | SLOT_INLINE => {
                    if slot_hash != kh {
                        continue;
                    }
                    if key_len as usize != key.len() {
                        continue;
                    }
                    let ho = head_off as usize;
                    let matches = self
                        .buf
                        .get(ho..)
                        .and_then(read_record_header)
                        .and_then(|hdr| record_key(&hdr, &self.buf[ho..]))
                        .map(|k| k == key)
                        .unwrap_or(false);
                    if !matches {
                        continue;
                    }

                    return self
                        .write_update(idx, tag, key, val, kh, bump, is_scalar, head_off)
                        .map(|_| InsertResult::Updated);
                }
                _ => {}
            }
        }

        if let Some(tomb_idx) = first_tomb {
            return self
                .write_new_entry(tomb_idx, key, val, kh, bump, is_scalar)
                .map(|_| InsertResult::Inserted);
        }
        Err(InsertError::TableFull)
    }

    /// Remove an entry by key.  Returns `true` if found and removed.
    pub fn remove(&mut self, key: &str) -> bool {
        let Some(_writer) = acquire_writer(self.buf) else {
            return false;
        };
        self.remove_locked(key)
    }

    fn remove_locked(&mut self, key: &str) -> bool {
        let kh = hash_key(self.hash_seed, key);
        let probe_start = kh as usize & self.mask;

        for probe in 0..self.num_buckets {
            let idx = (probe_start + probe) & self.mask;
            let (tag, _, key_len, slot_hash, head_off, _) = read_slot(self.buf, self.data_off, idx);
            match tag {
                SLOT_EMPTY => return false,
                SLOT_TOMBSTONE => continue,
                SLOT_OCCUPIED | SLOT_INLINE => {
                    if slot_hash != kh {
                        continue;
                    }
                    if key_len as usize != key.len() {
                        continue;
                    }
                    let ho = head_off as usize;
                    let matches = self
                        .buf
                        .get(ho..)
                        .and_then(read_record_header)
                        .and_then(|hdr| record_key(&hdr, &self.buf[ho..]))
                        .map(|k| k == key)
                        .unwrap_or(false);
                    if !matches {
                        continue;
                    }

                    let bump = header(self.buf).arena_bump.load(Ordering::Relaxed) as usize;
                    self.scratch.clear();
                    let rec_len =
                        encode_tombstone_record(key, idx as u32, kh, head_off, &mut self.scratch);
                    let new_head_off = if bump + rec_len <= self.arena_cap {
                        let abs = self.arena_start + bump;
                        self.buf[abs..abs + rec_len].copy_from_slice(&self.scratch);
                        header(self.buf)
                            .arena_bump
                            .store((bump + rec_len) as u32, Ordering::Release);
                        abs as u32
                    } else {
                        u32::MAX
                    };
                    commit_slot_head(self.buf, self.data_off, idx, SLOT_TOMBSTONE, new_head_off);
                    return true;
                }
                _ => {}
            }
        }
        false
    }

    // ── Private helpers ───────────────────────────────────

    /// Append a brand-new arena record and commit the slot for a fresh key.
    fn write_new_entry(
        &mut self,
        slot_idx: usize,
        key: &str,
        val: &Value<'_>,
        kh: u64,
        bump: usize,
        is_scalar: bool,
    ) -> Result<(), InsertError> {
        self.scratch.clear();
        let rec_len = if is_scalar {
            encode_put_inline_record(key, slot_idx as u32, kh, NO_PREV, &mut self.scratch)
        } else {
            encode_put_record(key, val, slot_idx as u32, kh, NO_PREV, &mut self.scratch)
        };
        if bump + rec_len > self.arena_cap {
            return Err(InsertError::ArenaFull);
        }
        let abs = self.arena_start + bump;
        self.buf[abs..abs + rec_len].copy_from_slice(&self.scratch);
        header(self.buf)
            .arena_bump
            .store((bump + rec_len) as u32, Ordering::Release);
        let head_off = abs as u32;

        if is_scalar {
            let Some((dtype_u8, inline_bytes)) = encode_inline_bytes(val) else {
                return Err(InsertError::InvalidValue);
            };
            commit_slot(
                self.buf,
                self.data_off,
                slot_idx,
                SLOT_INLINE,
                dtype_u8,
                key.len() as u32,
                kh,
                head_off,
                &inline_bytes,
            );
        } else {
            commit_slot(
                self.buf,
                self.data_off,
                slot_idx,
                SLOT_OCCUPIED,
                0,
                key.len() as u32,
                kh,
                head_off,
                &[0u8; 8],
            );
        }
        Ok(())
    }

    /// Update an existing key's slot (key already probed and confirmed).
    #[allow(clippy::too_many_arguments)]
    fn write_update(
        &mut self,
        slot_idx: usize,
        old_tag: u8,
        key: &str,
        val: &Value<'_>,
        kh: u64,
        bump: usize,
        is_scalar: bool,
        old_head_off: u32,
    ) -> Result<(), InsertError> {
        // Hot path: scalar → scalar update on INLINE slot — zero arena writes.
        if old_tag == SLOT_INLINE && is_scalar {
            let Some((dtype_u8, inline_bytes)) = encode_inline_bytes(val) else {
                return Err(InsertError::InvalidValue);
            };
            update_slot_inline_value(self.buf, self.data_off, slot_idx, dtype_u8, &inline_bytes);
            return Ok(());
        }

        // All other cases: append a new arena record.
        self.scratch.clear();
        let rec_len = if is_scalar {
            encode_put_inline_record(key, slot_idx as u32, kh, old_head_off, &mut self.scratch)
        } else {
            encode_put_record(
                key,
                val,
                slot_idx as u32,
                kh,
                old_head_off,
                &mut self.scratch,
            )
        };
        if bump + rec_len > self.arena_cap {
            return Err(InsertError::ArenaFull);
        }
        let abs = self.arena_start + bump;
        self.buf[abs..abs + rec_len].copy_from_slice(&self.scratch);
        header(self.buf)
            .arena_bump
            .store((bump + rec_len) as u32, Ordering::Release);
        let new_head_off = abs as u32;

        if is_scalar {
            let Some((dtype_u8, inline_bytes)) = encode_inline_bytes(val) else {
                return Err(InsertError::InvalidValue);
            };
            // key_len is known from `key.len()` (key match was verified in the probe loop).
            commit_slot(
                self.buf,
                self.data_off,
                slot_idx,
                SLOT_INLINE,
                dtype_u8,
                key.len() as u32,
                kh,
                new_head_off,
                &inline_bytes,
            );
        } else {
            commit_slot_head(
                self.buf,
                self.data_off,
                slot_idx,
                SLOT_OCCUPIED,
                new_head_off,
            );
        }
        Ok(())
    }
}

// ── Convenience constructors ──────────────────────────────

pub fn writer_from_buf(buf: &mut [u8]) -> Result<MemhWriter<'_>, MemhValidateError> {
    MemhWriter::new(buf)
}

pub fn view_from_buf(buf: &[u8]) -> Result<MemhView<'_>, MemhValidateError> {
    MemhView::new(buf)
}

// ── Compaction ────────────────────────────────────────────

/// Copy all live entries from `src_buf` into a freshly initialised `dst_buf`.
///
/// The destination buffer must be at least `required_total_size(num_buckets, new_arena_cap)`
/// bytes.  `dst_buf` is cleared and re-initialised before writing.  Returns the number
/// of entries copied.
pub fn compact(
    src_buf: &[u8],
    dst_buf: &mut [u8],
    num_buckets: u32,
    new_arena_cap: usize,
    hash_seed: u64,
) -> Result<usize, MemhInitError> {
    init_buf(dst_buf, num_buckets, new_arena_cap, hash_seed)?;
    let src = MemhView::new(src_buf).map_err(|_| MemhInitError::ArenaTooSmall)?;
    let mut dst = MemhWriter::new(dst_buf).map_err(|_| MemhInitError::ArenaTooSmall)?;

    let mut count = 0usize;
    // iter() yields only live entries in arena order.
    for (k, v) in src.iter() {
        let val = typed_value_to_value(&v);
        dst.insert(k, &val)
            .map_err(|_| MemhInitError::ArenaTooSmall)?;
        count += 1;
    }
    Ok(count)
}

/// Convert a `TypedValue` reference back to a `Value` for re-insertion.
fn typed_value_to_value<'a>(tv: &'a TypedValue<'a>) -> Value<'a> {
    match tv {
        TypedValue::U8(v) => Value::U8(*v),
        TypedValue::I32(v) => Value::I32(*v),
        TypedValue::I64(v) => Value::I64(*v),
        TypedValue::F32(v) => Value::F32(*v),
        TypedValue::F64(v) => Value::F64(*v),
        TypedValue::U64(v) => Value::U64(*v),
        TypedValue::U32(v) => Value::U32(*v),
        TypedValue::Str(s) => Value::Str(s),
        TypedValue::Bytes(b) => Value::Bytes(b),
    }
}

// ── Thread-safe writer ────────────────────────────────────

/// Thread-safe wrapper around an MEMH buffer for concurrent multi-threaded writing.
///
/// Internally uses a `Mutex<Vec<u8>>` so all write operations are serialised.
/// The buffer is validated once at construction; subsequent `insert`/`remove`
/// calls skip the O(num_buckets) re-validation.
///
/// # Example
/// ```rust,ignore
/// use std::sync::Arc;
/// let writer = Arc::new(SharedMemhWriter::new(buf)?);
/// let w = Arc::clone(&writer);
/// std::thread::spawn(move || { w.insert("k", &Value::U64(1)).unwrap(); });
/// ```
pub struct SharedMemhWriter {
    buf: std::sync::Mutex<Vec<u8>>,
}

impl SharedMemhWriter {
    /// Validate `buf` and wrap it in a thread-safe writer.
    pub fn new(buf: Vec<u8>) -> Result<Self, MemhValidateError> {
        validate_memh(&buf)?;
        Ok(Self {
            buf: std::sync::Mutex::new(buf),
        })
    }

    fn lock_buf(&self) -> std::sync::MutexGuard<'_, Vec<u8>> {
        lock_mutex(&self.buf, "SharedMemhWriter")
    }

    /// Insert or update `key` → `val`.  Acquires the internal mutex for the
    /// duration of the operation.
    pub fn insert(&self, key: &str, val: &Value<'_>) -> Result<InsertResult, InsertError> {
        let mut guard = self.lock_buf();
        // SAFETY: buffer was validated in new(); Mutex provides exclusive access.
        unsafe { MemhWriter::new_unchecked(&mut guard) }.insert(key, val)
    }

    /// Remove `key`.  Acquires the internal mutex.
    pub fn remove(&self, key: &str) -> bool {
        let mut guard = self.lock_buf();
        unsafe { MemhWriter::new_unchecked(&mut guard) }.remove(key)
    }

    /// Snapshot-read: calls `f` with a [`MemhView`] built from the locked buffer.
    ///
    /// Holds the mutex for the duration of `f`; keep the closure short.
    pub fn with_view<R>(&self, f: impl FnOnce(&MemhView<'_>) -> R) -> Result<R, MemhValidateError> {
        let guard = self.lock_buf();
        let view = MemhView::new(&guard).inspect_err(|e| {
            log::error!("SharedMemhWriter: corrupt buffer: {e}");
        })?;
        Ok(f(&view))
    }

    /// Access the raw buffer under the mutex.
    pub fn with_buf<R>(&self, f: impl FnOnce(&[u8]) -> R) -> R {
        let guard = self.lock_buf();
        f(&guard)
    }
}

// SAFETY: the internal Mutex makes all accesses exclusive; Vec<u8> is Send.
unsafe impl Send for SharedMemhWriter {}
unsafe impl Sync for SharedMemhWriter {}

// ── Tests ─────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::super::layout::required_total_size;
    use super::*;
    use crate::schema::Value;

    fn make_buf(buckets: u32, arena: usize) -> Vec<u8> {
        let mut buf = vec![0u8; required_total_size(buckets, arena)];
        init_buf(&mut buf, buckets, arena, 0).unwrap();
        buf
    }

    // ── Basic correctness ─────────────────────────────────

    #[test]
    fn init_and_validate_reject_misaligned_buffers() {
        let size = required_total_size(16, 1024);
        let mut storage = vec![0u8; size + 1];
        let misaligned = &mut storage[1..];
        assert!(matches!(
            init_buf(misaligned, 16, 1024, 0),
            Err(MemhInitError::MisalignedBuffer { .. })
        ));
        assert!(matches!(
            validate_memh(misaligned),
            Err(MemhValidateError::MisalignedBuffer { .. })
        ));
    }

    #[test]
    fn validate_rejects_corrupt_geometry_before_slot_access() {
        let mut buf = make_buf(16, 1024);
        header_mut(&mut buf).data_offset = u32::MAX;
        assert!(matches!(
            validate_memh(&buf),
            Err(MemhValidateError::InvalidLayout("invalid data_offset"))
        ));

        let mut buf = make_buf(16, 1024);
        super::super::layout::meta_mut(&mut buf).arena_start = u32::MAX;
        assert!(matches!(
            validate_memh(&buf),
            Err(MemhValidateError::InvalidLayout("invalid arena_start"))
        ));
    }

    #[test]
    fn insert_and_get_scalar() {
        let mut buf = make_buf(16, 1024);
        writer_from_buf(&mut buf)
            .unwrap()
            .insert("answer", &Value::I64(42))
            .unwrap();
        assert_eq!(
            view_from_buf(&buf).unwrap().get("answer"),
            Some(TypedValue::I64(42))
        );
    }

    #[test]
    fn insert_and_get_str_value() {
        let mut buf = make_buf(16, 2048);
        writer_from_buf(&mut buf)
            .unwrap()
            .insert("greeting", &Value::Str("hello"))
            .unwrap();
        assert_eq!(
            view_from_buf(&buf).unwrap().get("greeting"),
            Some(TypedValue::Str("hello"))
        );
    }

    #[test]
    fn insert_update_scalar() {
        let mut buf = make_buf(16, 4096);
        let mut w = writer_from_buf(&mut buf).unwrap();
        assert_eq!(
            w.insert("k", &Value::U32(1)).unwrap(),
            InsertResult::Inserted
        );
        assert_eq!(
            w.insert("k", &Value::U32(2)).unwrap(),
            InsertResult::Updated
        );
        drop(w);
        assert_eq!(
            view_from_buf(&buf).unwrap().get("k"),
            Some(TypedValue::U32(2))
        );
    }

    #[test]
    fn remove_key() {
        let mut buf = make_buf(16, 1024);
        let mut w = writer_from_buf(&mut buf).unwrap();
        w.insert("x", &Value::F64(3.14)).unwrap();
        assert!(w.remove("x"));
        drop(w);
        assert!(view_from_buf(&buf).unwrap().get("x").is_none());
    }

    #[test]
    fn tombstone_probe_chain_intact() {
        let mut buf = make_buf(8, 4096);
        {
            let mut w = writer_from_buf(&mut buf).unwrap();
            w.insert("a", &Value::I64(1)).unwrap();
            w.insert("b", &Value::I64(2)).unwrap();
            w.remove("a");
            w.insert("c", &Value::I64(3)).unwrap();
        }
        let v = view_from_buf(&buf).unwrap();
        assert!(v.get("a").is_none());
        assert_eq!(v.get("b"), Some(TypedValue::I64(2)));
        assert_eq!(v.get("c"), Some(TypedValue::I64(3)));
    }

    #[test]
    fn mixed_inline_and_arena_entries() {
        let mut buf = make_buf(16, 4096);
        {
            let mut w = writer_from_buf(&mut buf).unwrap();
            w.insert("latency_ms", &Value::F64(1.5)).unwrap();
            w.insert("host", &Value::Str("node-1")).unwrap();
            w.insert("count", &Value::U64(99)).unwrap();
        }
        let v = view_from_buf(&buf).unwrap();
        assert_eq!(v.get("latency_ms"), Some(TypedValue::F64(1.5)));
        assert_eq!(v.get("host"), Some(TypedValue::Str("node-1")));
        assert_eq!(v.get("count"), Some(TypedValue::U64(99)));
        assert_eq!(v.len(), 3);
    }

    #[test]
    fn validate_detects_wrong_magic() {
        let mut buf = make_buf(8, 512);
        buf[0] = 0xFF;
        assert!(matches!(
            validate_memh(&buf),
            Err(MemhValidateError::WrongMagic(_))
        ));
    }

    // ── MEMH v5 specific tests ────────────────────────────

    #[test]
    fn scalar_update_zero_arena_growth() {
        let mut buf = make_buf(16, 512);
        {
            let mut w = writer_from_buf(&mut buf).unwrap();
            w.insert("counter", &Value::U64(1)).unwrap();
        }
        let bump_after_insert = header(&buf).arena_bump.load(Ordering::Relaxed);
        {
            let mut w = writer_from_buf(&mut buf).unwrap();
            assert_eq!(
                w.insert("counter", &Value::U64(2)).unwrap(),
                InsertResult::Updated
            );
        }
        let bump_after_update = header(&buf).arena_bump.load(Ordering::Relaxed);
        assert_eq!(
            bump_after_insert, bump_after_update,
            "scalar update must not grow the arena"
        );
        assert_eq!(
            view_from_buf(&buf).unwrap().get("counter"),
            Some(TypedValue::U64(2))
        );
    }

    #[test]
    fn iter_yields_all_live_entries() {
        let mut buf = make_buf(16, 4096);
        {
            let mut w = writer_from_buf(&mut buf).unwrap();
            for i in 0u32..5 {
                w.insert(&format!("key{i}"), &Value::U64(i as u64 * 10))
                    .unwrap();
            }
        }
        let v = view_from_buf(&buf).unwrap();
        let mut pairs: Vec<_> = v.iter().map(|(k, val)| (k.to_owned(), val)).collect();
        pairs.sort_by_key(|(k, _)| k.clone());
        assert_eq!(pairs.len(), 5);
        for (i, (k, val)) in pairs.iter().enumerate() {
            assert_eq!(k, &format!("key{i}"));
            assert_eq!(*val, TypedValue::U64(i as u64 * 10));
        }
    }

    #[test]
    fn iter_only_yields_latest_version() {
        let mut buf = make_buf(16, 4096);
        {
            let mut w = writer_from_buf(&mut buf).unwrap();
            w.insert("x", &Value::Str("v1")).unwrap();
            w.insert("x", &Value::Str("v2")).unwrap();
            w.insert("x", &Value::Str("v3")).unwrap();
        }
        let v = view_from_buf(&buf).unwrap();
        let pairs: Vec<_> = v.iter().collect();
        assert_eq!(pairs.len(), 1, "iter must yield only the latest version");
        assert_eq!(pairs[0].1, TypedValue::Str("v3"));
    }

    #[test]
    fn remove_then_iter_skips_tombstone() {
        let mut buf = make_buf(16, 4096);
        {
            let mut w = writer_from_buf(&mut buf).unwrap();
            w.insert("keep", &Value::I64(1)).unwrap();
            w.insert("del", &Value::I64(2)).unwrap();
            w.remove("del");
        }
        let v = view_from_buf(&buf).unwrap();
        let keys: Vec<_> = v.iter().map(|(k, _)| k).collect();
        assert_eq!(keys, vec!["keep"]);
        assert_eq!(v.len(), 1);
    }

    #[test]
    fn type_change_scalar_to_str() {
        let mut buf = make_buf(16, 4096);
        {
            let mut w = writer_from_buf(&mut buf).unwrap();
            w.insert("key", &Value::I64(100)).unwrap();
            w.insert("key", &Value::Str("now-a-string")).unwrap();
        }
        let v = view_from_buf(&buf).unwrap();
        assert_eq!(v.get("key"), Some(TypedValue::Str("now-a-string")));
        let pairs: Vec<_> = v.iter().collect();
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].1, TypedValue::Str("now-a-string"));
    }

    #[test]
    fn compact_reduces_arena_size() {
        let mut buf = make_buf(16, 8192);
        {
            let mut w = writer_from_buf(&mut buf).unwrap();
            for i in 0..10u32 {
                w.insert("x", &Value::Str(&format!("val{i}"))).unwrap();
            }
            w.insert("y", &Value::U64(42)).unwrap();
        }
        let bump_before = header(&buf).arena_bump.load(Ordering::Relaxed);

        let mut dst = vec![0u8; required_total_size(16, 4096)];
        let n = compact(&buf, &mut dst, 16, 4096, 0).unwrap();
        let bump_after = header(&dst).arena_bump.load(Ordering::Relaxed);

        assert_eq!(n, 2, "should compact to 2 live entries");
        assert!(
            bump_after < bump_before,
            "compacted arena should be smaller"
        );

        let v = view_from_buf(&dst).unwrap();
        assert_eq!(v.get("x"), Some(TypedValue::Str("val9")));
        assert_eq!(v.get("y"), Some(TypedValue::U64(42)));
    }

    // ── SharedMemhWriter multi-thread tests ───────────────

    #[test]
    fn shared_writer_basic() {
        let buf = make_buf(64, 4096);
        let sw = SharedMemhWriter::new(buf).unwrap();
        sw.insert("k1", &Value::U64(1)).unwrap();
        sw.insert("k2", &Value::I32(-5)).unwrap();
        sw.with_view(|v| {
            assert_eq!(v.get("k1"), Some(TypedValue::U64(1)));
            assert_eq!(v.get("k2"), Some(TypedValue::I32(-5)));
        })
        .unwrap();
    }

    #[test]
    fn shared_writer_concurrent_inserts() {
        use std::sync::Arc;

        const N_THREADS: usize = 8;
        const PER_THREAD: usize = 64;

        let buf = make_buf(1024, 128 * 1024);
        let sw = Arc::new(SharedMemhWriter::new(buf).unwrap());

        let handles: Vec<_> = (0..N_THREADS)
            .map(|t| {
                let sw = Arc::clone(&sw);
                std::thread::spawn(move || {
                    for i in 0..PER_THREAD {
                        let key = format!("t{t}_k{i}");
                        sw.insert(&key, &Value::U64((t * PER_THREAD + i) as u64))
                            .unwrap();
                    }
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }

        sw.with_view(|v| {
            assert_eq!(v.len(), N_THREADS * PER_THREAD);
            for t in 0..N_THREADS {
                for i in 0..PER_THREAD {
                    let key = format!("t{t}_k{i}");
                    let expected = TypedValue::U64((t * PER_THREAD + i) as u64);
                    assert_eq!(v.get(&key), Some(expected), "missing {key}");
                }
            }
        })
        .unwrap();
    }

    #[test]
    fn shared_writer_concurrent_updates() {
        use std::sync::Arc;

        const N_THREADS: usize = 4;
        const ROUNDS: usize = 50;

        // Pre-populate shared keys that all threads will hammer.
        let mut buf = make_buf(256, 64 * 1024);
        {
            let mut w = writer_from_buf(&mut buf).unwrap();
            for k in 0..16u32 {
                w.insert(&format!("shared{k}"), &Value::U64(0)).unwrap();
            }
        }

        let sw = Arc::new(SharedMemhWriter::new(buf).unwrap());

        let handles: Vec<_> = (0..N_THREADS)
            .map(|t| {
                let sw = Arc::clone(&sw);
                std::thread::spawn(move || {
                    for round in 0..ROUNDS {
                        for k in 0..16u32 {
                            sw.insert(
                                &format!("shared{k}"),
                                &Value::U64((t * ROUNDS + round) as u64),
                            )
                            .unwrap();
                        }
                    }
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }

        // We don't assert exact values (last-write-wins is non-deterministic),
        // but the table must be internally consistent: every key must be present.
        sw.with_view(|v| {
            for k in 0..16u32 {
                assert!(
                    v.get(&format!("shared{k}")).is_some(),
                    "key shared{k} missing"
                );
            }
        })
        .unwrap();
    }
}
