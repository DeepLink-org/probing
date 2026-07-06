use crate::error::{MemtableError, Result};
use crate::layout::{
    chunk_header, col_desc, col_desc_mut, compute_data_offset, header, header_mut, r32, w32,
    ChunkHeader, ChunkState, Header, BYTE_ORDER_MARK, CHUNK_HEADER_SIZE, FLAGS_KNOWN, FLAG_DEDUP,
    MAGIC, TS_MAX_INIT, TS_MIN_INIT, VERSION,
};
use crate::schema::{DType, Schema, Value};
use std::mem;
use std::sync::atomic::Ordering;

/// Column names recognised as the designated timestamp column (must be
/// `I64`). Matched at [`init_buf`] time and recorded in `Header::ts_col`.
pub(crate) const TS_COL_NAMES: [&str; 2] = ["timestamp", "ts"];

/// Fold a committed row's timestamp into the chunk's `min_ts`/`max_ts`.
///
/// Called by the (single, lock-holding) writer **before** the `used`
/// Release store that publishes the row, so any reader that observes the
/// row also observes a covering ts range.
pub(crate) fn note_row_ts(ch: &ChunkHeader, ts: i64) {
    if ts < ch.min_ts.load(Ordering::Relaxed) {
        ch.min_ts.store(ts, Ordering::Relaxed);
    }
    if ts > ch.max_ts.load(Ordering::Relaxed) {
        ch.max_ts.store(ts, Ordering::Relaxed);
    }
}

/// Extract the designated timestamp from a row, per `Header::ts_col`.
#[inline]
pub(crate) fn row_ts(h: &Header, values: &[Value]) -> Option<i64> {
    match h.ts_col as usize {
        0 => None,
        idx => match values.get(idx - 1) {
            Some(Value::I64(ts)) => Some(*ts),
            _ => None,
        },
    }
}

/// Returns the kernel-reported start time of a process.
///
/// Used to populate [`Header::creator_start_time`] and to verify liveness
/// during discovery (detecting PID recycling).
///
/// - **Linux**: clock ticks since boot from `/proc/<pid>/stat` field 22.
/// - **macOS**: microseconds since epoch via `sysctl(KERN_PROC_PID)`.
/// - **Other**: returns 0 (graceful degradation to PID-only check).
#[cfg(target_os = "linux")]
pub(crate) fn process_start_time(pid: u32) -> u64 {
    let path = if pid == std::process::id() {
        "/proc/self/stat".to_string()
    } else {
        format!("/proc/{}/stat", pid)
    };
    if let Ok(stat) = std::fs::read_to_string(path) {
        if let Some(pos) = stat.rfind(')') {
            let rest = &stat[pos + 2..];
            if let Some(time_str) = rest.split_whitespace().nth(19) {
                if let Ok(time) = time_str.parse::<u64>() {
                    return time;
                }
            }
        }
    }
    0
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn process_start_time(_pid: u32) -> u64 {
    // macOS / other: graceful degradation to PID-only liveness check.
    // is_creator_alive() skips start-time comparison when this returns 0.
    0
}

/// Try to append a row whose total payload size (`row_data`) is already known.
/// Caller must validate schema and compute `row_data` before calling.
/// Returns `false` if the current chunk has no room.
pub(crate) fn write_row_bytes(buf: &mut [u8], values: &[Value], row_data: usize) -> bool {
    let ptr = buf.as_mut_ptr();
    let (wc, csz, doff) = unsafe {
        let h = &*(ptr as *const Header);
        (
            h.write_chunk.load(Ordering::Relaxed) as usize,
            h.chunk_size as usize,
            h.data_offset as usize,
        )
    };
    let cs = doff + wc * csz;
    let used = unsafe {
        let ch = &*(ptr.add(cs) as *const ChunkHeader);
        ch.used.load(Ordering::Relaxed) as usize
    };

    let total = 4 + row_data;
    if CHUNK_HEADER_SIZE + used + total > csz {
        return false;
    }

    let row_start = cs + CHUNK_HEADER_SIZE + used;
    w32(buf, row_start, row_data as u32);
    let mut off = row_start + 4;
    for v in values {
        off += v.encode(&mut buf[off..]);
    }
    unsafe {
        let ch = &*(ptr.add(cs) as *const ChunkHeader);
        if let Some(ts) = row_ts(&*(ptr as *const Header), values) {
            note_row_ts(ch, ts);
        }
        ch.used.store((used + total) as u32, Ordering::Release);
        ch.row_count.fetch_add(1, Ordering::Release);
    }
    true
}

/// Advance the ring buffer to the next chunk.
///
/// MEMT is single-writer, so no lock is taken. Takes `&mut [u8]` so that
/// LLVM does not mark the pointer `readonly` (which would let it elide the
/// atomic stores below in optimised builds).
pub(crate) fn advance_chunk_raw(buf: &mut [u8]) {
    let ptr = buf.as_mut_ptr();
    unsafe {
        let h = &*(ptr as *const Header);
        let wc = h.write_chunk.load(Ordering::Relaxed);
        let csz = h.chunk_size as usize;
        let doff = h.data_offset as usize;
        let num_chunks = h.num_chunks;

        let cur_cs = doff + wc as usize * csz;
        let cur_ch = &*(ptr.add(cur_cs) as *const ChunkHeader);
        cur_ch
            .state
            .store(ChunkState::Sealed as u32, Ordering::Release);

        let new_wc = (wc + 1) % num_chunks;
        let cs = doff + new_wc as usize * csz;
        let new_ch = &*(ptr.add(cs) as *const ChunkHeader);
        let rows_lost = new_ch.row_count.load(Ordering::Relaxed);
        if rows_lost > 0 {
            h.chunks_recycled.fetch_add(1, Ordering::Relaxed);
            h.rows_overwritten.fetch_add(rows_lost, Ordering::Relaxed);
            log::warn!(
                "memtable ring overwrite: lost {rows_lost} rows (total overwritten {})",
                h.rows_overwritten.load(Ordering::Relaxed)
            );
        }
        new_ch.used.store(0, Ordering::Relaxed);
        new_ch.row_count.store(0, Ordering::Relaxed);
        new_ch.min_ts.store(TS_MIN_INIT, Ordering::Relaxed);
        new_ch.max_ts.store(TS_MAX_INIT, Ordering::Relaxed);
        new_ch
            .state
            .store(ChunkState::Writing as u32, Ordering::Relaxed);
        // Generation bump LAST with Release: readers that Acquire this
        // new generation are guaranteed to see the zeroed used/state above.
        new_ch.generation.fetch_add(1, Ordering::Release);

        (&*(ptr as *const Header))
            .write_chunk
            .store(new_wc, Ordering::Release);
    }
}
/// Walk rows in a sealed chunk: verify row lengths stay within `used` and
/// dedup refs (negative var-length prefix) point inside the chunk data region.
///
/// When `has_dedup` is false, any negative length prefix is rejected as
/// invalid — the buffer was not written with dedup enabled.
fn validate_chunk_rows(
    buf: &[u8],
    cs: usize,
    used: usize,
    nc: usize,
    has_dedup: bool,
) -> Result<()> {
    let data_base = cs + CHUNK_HEADER_SIZE;
    let mut pos = 0usize;
    while pos + 4 <= used {
        let row_len = r32(buf, data_base + pos) as usize;
        if pos + 4 + row_len > used {
            return Err(MemtableError::InvalidBuffer(
                "row extends beyond chunk used region",
            ));
        }
        let row_start = data_base + pos + 4;
        let mut col_off = 0usize;
        for ci in 0..nc {
            if col_off >= row_len {
                break;
            }
            let Some(dt) = DType::from_u32(col_desc(buf, ci).dtype) else {
                break;
            };
            if let Some(sz) = dt.fixed_size() {
                col_off += sz;
            } else if col_off + 4 <= row_len {
                let raw = i32::from_le_bytes(
                    buf[row_start + col_off..row_start + col_off + 4]
                        .try_into()
                        .unwrap(),
                );
                if raw < 0 {
                    if !has_dedup {
                        return Err(MemtableError::InvalidBuffer("dedup ref in non-dedup table"));
                    }
                    let ref_off = (-raw) as usize;
                    if ref_off < CHUNK_HEADER_SIZE || ref_off >= CHUNK_HEADER_SIZE + used {
                        return Err(MemtableError::InvalidBuffer(
                            "dedup ref outside chunk data region",
                        ));
                    }
                    col_off += 4;
                } else {
                    col_off += 4 + raw as usize;
                }
            }
        }
        pos += 4 + row_len;
    }
    Ok(())
}

/// Structural validation of a MemTable buffer.
///
/// Checks magic, version, byte order, feature flags, layout offsets,
/// column dtypes, chunk states, used-within-payload bounds, row boundary
/// integrity, and dedup ref ranges.
///
/// All `from_buf` / `new` constructors funnel through this function.
pub fn validate_buf(buf: &[u8]) -> Result<()> {
    if buf.len() < mem::size_of::<Header>() {
        return Err(MemtableError::InvalidBuffer("buffer too small for header"));
    }
    let h = header(buf);
    if h.magic != MAGIC {
        return Err(MemtableError::InvalidBuffer("invalid magic"));
    }
    if h.version != VERSION {
        return Err(MemtableError::InvalidBuffer("unsupported version"));
    }
    if (h.header_size as usize) < mem::size_of::<Header>() {
        return Err(MemtableError::InvalidBuffer("header_size too small"));
    }
    let bom = u16::from_ne_bytes(BYTE_ORDER_MARK);
    if h.byte_order != bom {
        return Err(MemtableError::InvalidBuffer(
            "byte order mismatch (buffer written on different-endian host)",
        ));
    }
    if h.flags & !FLAGS_KNOWN != 0 {
        return Err(MemtableError::InvalidBuffer("unknown feature flags set"));
    }
    let has_dedup = h.flags & FLAG_DEDUP != 0;
    let nc = h.num_cols as usize;
    if h.num_chunks == 0 {
        return Err(MemtableError::InvalidBuffer("num_chunks must be > 0"));
    }
    let csz = h.chunk_size as usize;
    if csz < CHUNK_HEADER_SIZE + 8 {
        return Err(MemtableError::InvalidBuffer("chunk_size too small"));
    }
    let expected_off = compute_data_offset(nc);
    if h.data_offset as usize != expected_off {
        return Err(MemtableError::InvalidBuffer("invalid data_offset"));
    }
    let required = expected_off + csz * h.num_chunks as usize;
    if buf.len() < required {
        return Err(MemtableError::InvalidBuffer("buffer too small for data"));
    }
    for i in 0..nc {
        let dt = col_desc(buf, i).dtype;
        if !(1..=9).contains(&dt) {
            return Err(MemtableError::InvalidBuffer("invalid column dtype"));
        }
    }
    let ts_col = h.ts_col as usize;
    if ts_col != 0 {
        if ts_col > nc {
            return Err(MemtableError::InvalidBuffer("ts_col out of range"));
        }
        if DType::from_u32(col_desc(buf, ts_col - 1).dtype) != Some(DType::I64) {
            return Err(MemtableError::InvalidBuffer(
                "ts_col must reference an I64 column",
            ));
        }
    }
    let payload_cap = csz - CHUNK_HEADER_SIZE;
    for i in 0..h.num_chunks as usize {
        let cs = expected_off + i * csz;
        let ch = chunk_header(buf, cs);
        let state = ch.state.load(Ordering::Acquire);
        if state > 2 {
            return Err(MemtableError::InvalidBuffer("invalid chunk state"));
        }
        let used = ch.used.load(Ordering::Acquire) as usize;
        if used > payload_cap {
            return Err(MemtableError::InvalidBuffer(
                "chunk used exceeds payload capacity",
            ));
        }
        if state == ChunkState::Sealed as u32 && used > 0 {
            let gen_before = ch.generation.load(Ordering::Acquire);
            let snap_used = ch.used.load(Ordering::Acquire) as usize;
            if snap_used > 0 && snap_used <= payload_cap {
                let result = validate_chunk_rows(buf, cs, snap_used, nc, has_dedup);
                if ch.generation.load(Ordering::Acquire) == gen_before {
                    result?;
                }
            }
        }
    }
    Ok(())
}

/// Check that `values` matches the table schema (column count + dtypes).
pub(crate) fn validate_row_schema(buf: &[u8], values: &[Value]) -> bool {
    let nc = header(buf).num_cols as usize;
    if values.len() != nc {
        return false;
    }
    for (i, v) in values.iter().enumerate() {
        let Some(dt) = DType::from_u32(col_desc(buf, i).dtype) else {
            return false;
        };
        let ok = matches!(
            (v, dt),
            (Value::U8(_), DType::U8)
                | (Value::U32(_), DType::U32)
                | (Value::I32(_), DType::I32)
                | (Value::I64(_), DType::I64)
                | (Value::F32(_), DType::F32)
                | (Value::F64(_), DType::F64)
                | (Value::U64(_), DType::U64)
                | (Value::Str(_), DType::Str)
                | (Value::Bytes(_), DType::Bytes)
        );
        if !ok {
            return false;
        }
    }
    true
}

// ── init ────────────────────────────────────────────────────────────

pub(crate) fn init_buf(buf: &mut [u8], schema: &Schema, chunk_size: u32, num_chunks: u32) {
    let nc = schema.cols.len();
    let data_off = compute_data_offset(nc);
    let required = data_off + chunk_size as usize * num_chunks as usize;
    assert!(
        buf.len() >= required,
        "buffer too small: need {required} bytes, got {}",
        buf.len()
    );
    assert!(
        chunk_size as usize >= CHUNK_HEADER_SIZE + 8,
        "chunk_size must be at least {} bytes",
        CHUNK_HEADER_SIZE + 8
    );

    // First I64 column with a recognised timestamp name becomes the
    // designated time column (index + 1; 0 = none).
    let ts_col = schema
        .cols
        .iter()
        .position(|c| c.dtype == DType::I64 && TS_COL_NAMES.contains(&c.name.as_str()))
        .map(|i| (i + 1) as u16)
        .unwrap_or(0);

    let h = header_mut(buf);
    h.magic = MAGIC;
    h.version = VERSION;
    h.header_size = mem::size_of::<Header>() as u16;
    h.byte_order = u16::from_ne_bytes(BYTE_ORDER_MARK);
    h.ts_col = ts_col;
    h.flags = 0;
    h.num_cols = nc as u32;
    h.num_chunks = num_chunks;
    h.chunk_size = chunk_size;
    h.data_offset = data_off as u32;
    h.write_chunk.store(0, Ordering::Relaxed);
    h.refcount.store(1, Ordering::Relaxed);
    h.creator_pid = std::process::id();
    h._pad0 = 0;
    h.creator_start_time = process_start_time(std::process::id());
    h.chunks_recycled.store(0, Ordering::Relaxed);
    h.rows_overwritten.store(0, Ordering::Relaxed);

    for (i, col) in schema.cols.iter().enumerate() {
        let cd = col_desc_mut(buf, i);
        cd.set_name(&col.name);
        cd.dtype = col.dtype as u32;
        cd.elem_size = col.elem_size as u32;
    }

    // Initialize all chunk headers
    for i in 0..num_chunks as usize {
        let cs = data_off + i * chunk_size as usize;
        let ch = chunk_header(buf, cs);
        ch.generation.store(0, Ordering::Relaxed);
        ch.used.store(0, Ordering::Relaxed);
        ch.row_count.store(0, Ordering::Relaxed);
        ch.min_ts.store(TS_MIN_INIT, Ordering::Relaxed);
        ch.max_ts.store(TS_MAX_INIT, Ordering::Relaxed);
        ch.state.store(ChunkState::Empty as u32, Ordering::Relaxed);
    }
    // Chunk 0 is the initial write target
    let ch0 = chunk_header(buf, data_off);
    ch0.generation.store(1, Ordering::Relaxed);
    ch0.state
        .store(ChunkState::Writing as u32, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memtable::push_plain_row;
    use crate::schema::{DType, Schema, Value};

    #[test]
    #[should_panic(expected = "buffer too small")]
    fn init_buf_rejects_small_buffer() {
        let schema = Schema::new().col("x", DType::I32);
        let mut buf = vec![0u8; 32]; // way too small
        init_buf(&mut buf, &schema, 1024, 1);
    }

    #[test]
    fn advance_chunk_counts_overwritten_rows() {
        let schema = Schema::new().col("x", DType::I32);
        let chunk_size = 128u32;
        let num_chunks = 2u32;
        let total = crate::layout::compute_data_offset(schema.cols.len())
            + chunk_size as usize * num_chunks as usize;
        let mut buf = vec![0u8; total];
        init_buf(&mut buf, &schema, chunk_size, num_chunks);

        for i in 0..10_000i32 {
            assert!(push_plain_row(&mut buf, &[Value::I32(i)]));
            let (_, rows) = crate::layout::ring_overwrite_stats(&buf);
            if rows > 0 {
                assert!(rows >= 1);
                return;
            }
        }
        panic!("expected ring overwrite within 10_000 pushes");
    }
}
