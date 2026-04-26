use crate::layout::{
    chunk_header, col_desc, col_desc_mut, compute_data_offset, header, header_mut, r32, w32,
    ChunkHeader, ChunkState, Header, BYTE_ORDER_MARK, CHUNK_HEADER_SIZE, FLAGS_KNOWN, FLAG_DEDUP,
    MAGIC, VERSION,
};
use crate::schema::{DType, Schema, Value};
use std::mem;
use std::sync::atomic::Ordering;

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
        ch.used.store((used + total) as u32, Ordering::Release);
        ch.row_count.fetch_add(1, Ordering::Release);
    }
    true
}

/// Advance the ring buffer to the next chunk (caller must hold the write lock).
///
/// Takes `&mut [u8]` so that LLVM does not mark the pointer `readonly`;
/// see [`acquire_write_lock`](crate::layout::acquire_write_lock) for details.
pub(crate) fn advance_chunk_unlocked(buf: &mut [u8]) {
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
        new_ch.used.store(0, Ordering::Relaxed);
        new_ch.row_count.store(0, Ordering::Relaxed);
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
) -> Result<(), &'static str> {
    let data_base = cs + CHUNK_HEADER_SIZE;
    let mut pos = 0usize;
    while pos + 4 <= used {
        let row_len = r32(buf, data_base + pos) as usize;
        if pos + 4 + row_len > used {
            return Err("row extends beyond chunk used region");
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
                        return Err("dedup ref in non-dedup table");
                    }
                    let ref_off = (-raw) as usize;
                    if ref_off < CHUNK_HEADER_SIZE || ref_off >= CHUNK_HEADER_SIZE + used {
                        return Err("dedup ref outside chunk data region");
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
pub fn validate_buf(buf: &[u8]) -> Result<(), &'static str> {
    if buf.len() < mem::size_of::<Header>() {
        return Err("buffer too small for header");
    }
    let h = header(buf);
    if h.magic != MAGIC {
        return Err("invalid magic");
    }
    if h.version != VERSION {
        return Err("unsupported version");
    }
    if (h.header_size as usize) < mem::size_of::<Header>() {
        return Err("header_size too small");
    }
    let bom = u16::from_ne_bytes(BYTE_ORDER_MARK);
    if h.byte_order != bom {
        return Err("byte order mismatch (buffer written on different-endian host)");
    }
    if h.flags & !FLAGS_KNOWN != 0 {
        return Err("unknown feature flags set");
    }
    let has_dedup = h.flags & FLAG_DEDUP != 0;
    let nc = h.num_cols as usize;
    if h.num_chunks == 0 {
        return Err("num_chunks must be > 0");
    }
    let csz = h.chunk_size as usize;
    if csz < CHUNK_HEADER_SIZE + 8 {
        return Err("chunk_size too small");
    }
    let expected_off = compute_data_offset(nc);
    if h.data_offset as usize != expected_off {
        return Err("invalid data_offset");
    }
    let required = expected_off + csz * h.num_chunks as usize;
    if buf.len() < required {
        return Err("buffer too small for data");
    }
    for i in 0..nc {
        let dt = col_desc(buf, i).dtype;
        if !(1..=9).contains(&dt) {
            return Err("invalid column dtype");
        }
    }
    let payload_cap = csz - CHUNK_HEADER_SIZE;
    for i in 0..h.num_chunks as usize {
        let cs = expected_off + i * csz;
        let ch = chunk_header(buf, cs);
        let state = ch.state.load(Ordering::Acquire);
        if state > 2 {
            return Err("invalid chunk state");
        }
        let used = ch.used.load(Ordering::Acquire) as usize;
        if used > payload_cap {
            return Err("chunk used exceeds payload capacity");
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

    let h = header_mut(buf);
    h.magic = MAGIC;
    h.version = VERSION;
    h.header_size = mem::size_of::<Header>() as u16;
    h.byte_order = u16::from_ne_bytes(BYTE_ORDER_MARK);
    h._pad0 = 0;
    h.flags = 0;
    h.num_cols = nc as u32;
    h.num_chunks = num_chunks;
    h.chunk_size = chunk_size;
    h.data_offset = data_off as u32;
    h.write_chunk.store(0, Ordering::Relaxed);
    h.write_lock.store(0, Ordering::Relaxed);
    h.refcount.store(1, Ordering::Relaxed);
    h.creator_pid = std::process::id();
    h.creator_start_time = process_start_time(std::process::id());
    h._reserved = [0; 2];

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
    use crate::schema::{DType, Schema};

    #[test]
    fn init_buf_rejects_small_buffer() {
        let schema = Schema::new().col("x", DType::I32);
        let result = std::panic::catch_unwind(|| {
            let mut buf = vec![0u8; 32]; // way too small
            init_buf(&mut buf, &schema, 1024, 1);
        });
        assert!(
            result.is_err(),
            "init_buf should panic on undersized buffer"
        );
    }
}
