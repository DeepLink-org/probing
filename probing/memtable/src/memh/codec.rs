//! Codec utilities for MEMH v3: hashing, arena record encoding/decoding.

use std::mem;
use xxhash_rust::xxh3;

use crate::schema::{DType, Value};

// ── Arena record flag constants ───────────────────────────

/// Record type: key + value payload in record.
pub const FLAG_PUT: u16 = 1;
/// Record type: key deleted; value in record payload should be ignored.
pub const FLAG_TOMBSTONE: u16 = 2;
/// Record type: key only in record payload; scalar value lives in slot.val_bytes.
pub const FLAG_PUT_INLINE: u16 = 3;

/// `val_dtype` sentinel for TOMBSTONE and PUT_INLINE records.
pub const DTYPE_NONE: u32 = 0xFFFF_FFFF;
/// `prev_off` sentinel meaning "no previous version".
pub const NO_PREV: u32 = 0xFFFF_FFFF;

// ── ArenaRecordHeader ─────────────────────────────────────

/// Fixed-size header (28 bytes) at the start of every arena record.
///
/// Layout (all fields little-endian):
/// ```text
///  +0   4B  record_len  total record bytes (header + payload, 4-byte aligned)
///  +4   4B  slot_idx    owning bucket index (for iter liveness check)
///  +8   4B  hash_lo     low 32 bits of xxh3_64(key)
/// +12   4B  hash_hi     high 32 bits of xxh3_64(key)
/// +16   4B  prev_off    absolute buf offset of previous version; NO_PREV = 0xFFFF_FFFF
/// +20   2B  flags       PUT=1 / TOMBSTONE=2 / PUT_INLINE=3
/// +22   2B  key_len     byte length of key
/// +24   4B  val_dtype   DType discriminant (0xFFFF_FFFF for PUT_INLINE / TOMBSTONE)
/// ```
/// Payload begins at offset 28: `[key_bytes][val_payload]`
/// (PUT_INLINE and TOMBSTONE have no val_payload.)
#[repr(C)]
pub struct ArenaRecordHeader {
    pub record_len: u32, //  0
    pub slot_idx: u32,   //  4
    pub hash_lo: u32,    //  8
    pub hash_hi: u32,    // 12
    pub prev_off: u32,   // 16
    pub flags: u16,      // 20
    pub key_len: u16,    // 22
    pub val_dtype: u32,  // 24
}

impl ArenaRecordHeader {
    /// Reconstruct the full 64-bit hash.
    #[inline]
    pub fn hash(&self) -> u64 {
        (self.hash_lo as u64) | ((self.hash_hi as u64) << 32)
    }
}

/// Byte size of `ArenaRecordHeader` — payload begins immediately after.
pub const ARENA_HDR_SIZE: usize = 28;

const _: () = assert!(mem::size_of::<ArenaRecordHeader>() == ARENA_HDR_SIZE);

// ── Zero-copy decoded value ───────────────────────────────

/// Zero-copy decoded value; borrows from the arena buffer.
#[derive(Debug, Clone, PartialEq)]
pub enum TypedValue<'a> {
    U8(u8),
    I32(i32),
    I64(i64),
    F32(f32),
    F64(f64),
    U64(u64),
    U32(u32),
    Str(&'a str),
    Bytes(&'a [u8]),
}

impl TypedValue<'_> {
    pub fn dtype(&self) -> DType {
        match self {
            TypedValue::U8(_) => DType::U8,
            TypedValue::I32(_) => DType::I32,
            TypedValue::I64(_) => DType::I64,
            TypedValue::F32(_) => DType::F32,
            TypedValue::F64(_) => DType::F64,
            TypedValue::U64(_) => DType::U64,
            TypedValue::U32(_) => DType::U32,
            TypedValue::Str(_) => DType::Str,
            TypedValue::Bytes(_) => DType::Bytes,
        }
    }
}

// ── Helpers ───────────────────────────────────────────────

/// Returns `true` for fixed-size scalar dtypes that can be stored inline in a slot.
#[inline]
pub fn is_scalar_dtype(dt: DType) -> bool {
    !matches!(dt, DType::Str | DType::Bytes)
}

/// Align `n` up to the next 4-byte boundary.
#[inline]
pub fn align4(n: usize) -> usize {
    (n + 3) & !3
}

// ── Hashing ───────────────────────────────────────────────

/// Hash a key string with the given seed using xxHash3-64.
pub fn hash_key(seed: u64, key: &str) -> u64 {
    xxh3::xxh3_64_with_seed(key.as_bytes(), seed)
}

// ── Scalar ↔ [u8;8] helpers ───────────────────────────────

/// Encode a scalar `Value` into the 8-byte inline slot storage (little-endian, zero-padded).
/// Returns `None` for Str/Bytes (not scalar).
pub fn encode_inline_bytes(val: &Value<'_>) -> Option<(u8, [u8; 8])> {
    let mut out = [0u8; 8];
    let dtype = val.dtype();
    if !is_scalar_dtype(dtype) {
        return None;
    }
    let n = val.encode(&mut out);
    debug_assert!(n <= 8);
    Some((dtype as u8, out))
}

/// Decode an inline scalar from `(val_dtype, val_bytes)`.
pub fn decode_inline_value(val_dtype: u8, val_bytes: &[u8; 8]) -> Option<TypedValue<'static>> {
    let dt = DType::from_u32(val_dtype as u32)?;
    let v = match dt {
        DType::U8 => TypedValue::U8(val_bytes[0]),
        DType::I32 => TypedValue::I32(i32::from_le_bytes(val_bytes[..4].try_into().unwrap())),
        DType::I64 => TypedValue::I64(i64::from_le_bytes(val_bytes[..8].try_into().unwrap())),
        DType::F32 => TypedValue::F32(f32::from_le_bytes(val_bytes[..4].try_into().unwrap())),
        DType::F64 => TypedValue::F64(f64::from_le_bytes(val_bytes[..8].try_into().unwrap())),
        DType::U64 => TypedValue::U64(u64::from_le_bytes(val_bytes[..8].try_into().unwrap())),
        DType::U32 => TypedValue::U32(u32::from_le_bytes(val_bytes[..4].try_into().unwrap())),
        DType::Str | DType::Bytes => return None,
    };
    Some(v)
}

// ── Value payload encode/decode ───────────────────────────

/// Decode a value payload (no dtype prefix) given `dtype` and the raw payload slice.
///
/// For `Str`/`Bytes` the payload is `[u32 len][bytes]`.
pub fn decode_value_payload<'a>(dtype: DType, payload: &'a [u8]) -> Option<TypedValue<'a>> {
    match dtype {
        DType::U8 => Some(TypedValue::U8(payload[0])),
        DType::I32 => Some(TypedValue::I32(i32::from_le_bytes(
            payload[..4].try_into().ok()?,
        ))),
        DType::I64 => Some(TypedValue::I64(i64::from_le_bytes(
            payload[..8].try_into().ok()?,
        ))),
        DType::F32 => Some(TypedValue::F32(f32::from_le_bytes(
            payload[..4].try_into().ok()?,
        ))),
        DType::F64 => Some(TypedValue::F64(f64::from_le_bytes(
            payload[..8].try_into().ok()?,
        ))),
        DType::U64 => Some(TypedValue::U64(u64::from_le_bytes(
            payload[..8].try_into().ok()?,
        ))),
        DType::U32 => Some(TypedValue::U32(u32::from_le_bytes(
            payload[..4].try_into().ok()?,
        ))),
        DType::Str => {
            let slen = u32::from_le_bytes(payload[..4].try_into().ok()?) as usize;
            let bytes = payload.get(4..4 + slen)?;
            Some(TypedValue::Str(std::str::from_utf8(bytes).ok()?))
        }
        DType::Bytes => {
            let slen = u32::from_le_bytes(payload[..4].try_into().ok()?) as usize;
            let bytes = payload.get(4..4 + slen)?;
            Some(TypedValue::Bytes(bytes))
        }
    }
}

// ── Arena record encode ───────────────────────────────────

fn write_hdr(
    out: &mut Vec<u8>,
    record_len: u32,
    slot_idx: u32,
    hash: u64,
    prev_off: u32,
    flags: u16,
    key_len: u16,
    val_dtype: u32,
) {
    // Build a 28-byte buffer on the stack and append it in one shot.
    // This is one extend_from_slice (one capacity check + one 28-byte copy)
    // instead of 8 separate calls.
    let mut hdr = [0u8; ARENA_HDR_SIZE];
    hdr[0..4].copy_from_slice(&record_len.to_le_bytes());
    hdr[4..8].copy_from_slice(&slot_idx.to_le_bytes());
    hdr[8..12].copy_from_slice(&(hash as u32).to_le_bytes());
    hdr[12..16].copy_from_slice(&((hash >> 32) as u32).to_le_bytes());
    hdr[16..20].copy_from_slice(&prev_off.to_le_bytes());
    hdr[20..22].copy_from_slice(&flags.to_le_bytes());
    hdr[22..24].copy_from_slice(&key_len.to_le_bytes());
    hdr[24..28].copy_from_slice(&val_dtype.to_le_bytes());
    out.extend_from_slice(&hdr);
}

/// Encode a PUT record (key + value payload) into `out`.
///
/// The caller is responsible for appending `out` to the arena at the correct
/// offset.  Returns the number of bytes written (always a multiple of 4).
pub fn encode_put_record(
    key: &str,
    val: &Value<'_>,
    slot_idx: u32,
    hash: u64,
    prev_off: u32,
    out: &mut Vec<u8>,
) -> usize {
    let key_bytes = key.as_bytes();
    let val_sz = val.encoded_size();
    let payload_len = key_bytes.len() + val_sz;
    let record_len = align4(ARENA_HDR_SIZE + payload_len) as u32;

    let start = out.len();
    write_hdr(
        out,
        record_len,
        slot_idx,
        hash,
        prev_off,
        FLAG_PUT,
        key_bytes.len() as u16,
        val.dtype() as u32,
    );
    out.extend_from_slice(key_bytes);
    let prev = out.len();
    out.resize(prev + val_sz, 0);
    val.encode(&mut out[prev..]);
    // pad to 4-byte alignment
    let cur_len = out.len() - start;
    out.resize(start + align4(cur_len), 0);
    align4(cur_len)
}

/// Encode a PUT_INLINE record (key only; value lives in slot.val_bytes).
pub fn encode_put_inline_record(
    key: &str,
    slot_idx: u32,
    hash: u64,
    prev_off: u32,
    out: &mut Vec<u8>,
) -> usize {
    let key_bytes = key.as_bytes();
    let record_len = align4(ARENA_HDR_SIZE + key_bytes.len()) as u32;

    let start = out.len();
    write_hdr(
        out,
        record_len,
        slot_idx,
        hash,
        prev_off,
        FLAG_PUT_INLINE,
        key_bytes.len() as u16,
        DTYPE_NONE,
    );
    out.extend_from_slice(key_bytes);
    let cur_len = out.len() - start;
    out.resize(start + align4(cur_len), 0);
    align4(cur_len)
}

/// Encode a TOMBSTONE record (key only; marks deletion).
pub fn encode_tombstone_record(
    key: &str,
    slot_idx: u32,
    hash: u64,
    prev_off: u32,
    out: &mut Vec<u8>,
) -> usize {
    let key_bytes = key.as_bytes();
    let record_len = align4(ARENA_HDR_SIZE + key_bytes.len()) as u32;

    let start = out.len();
    write_hdr(
        out,
        record_len,
        slot_idx,
        hash,
        prev_off,
        FLAG_TOMBSTONE,
        key_bytes.len() as u16,
        DTYPE_NONE,
    );
    out.extend_from_slice(key_bytes);
    let cur_len = out.len() - start;
    out.resize(start + align4(cur_len), 0);
    align4(cur_len)
}

// ── Arena record decode ───────────────────────────────────

/// Read the record header at the start of `data` (which must be at least 28 bytes).
/// Returns `None` if `data` is too short.
pub fn read_record_header(data: &[u8]) -> Option<&ArenaRecordHeader> {
    if data.len() < ARENA_HDR_SIZE {
        return None;
    }
    Some(unsafe { &*(data.as_ptr() as *const ArenaRecordHeader) })
}

/// Return the key bytes of the record starting at `record_data` (from record start).
pub fn record_key<'a>(hdr: &ArenaRecordHeader, record_data: &'a [u8]) -> Option<&'a str> {
    let klen = hdr.key_len as usize;
    let key_bytes = record_data.get(ARENA_HDR_SIZE..ARENA_HDR_SIZE + klen)?;
    std::str::from_utf8(key_bytes).ok()
}

/// Decode and return the value payload of a PUT record.
///
/// Returns `None` for TOMBSTONE and PUT_INLINE records (those have no value payload).
pub fn record_value<'a>(hdr: &ArenaRecordHeader, record_data: &'a [u8]) -> Option<TypedValue<'a>> {
    if hdr.flags != FLAG_PUT {
        return None;
    }
    let dtype = DType::from_u32(hdr.val_dtype)?;
    let key_end = ARENA_HDR_SIZE + hdr.key_len as usize;
    let val_payload = record_data.get(key_end..hdr.record_len as usize)?;
    decode_value_payload(dtype, val_payload)
}

/// Decode an inline scalar from a slot's `val_dtype` and `val_bytes` fields.
pub fn slot_inline_value(val_dtype: u8, val_bytes: &[u8; 8]) -> Option<TypedValue<'static>> {
    decode_inline_value(val_dtype, val_bytes)
}
