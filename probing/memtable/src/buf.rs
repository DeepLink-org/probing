use crate::layout::{
    chunk_header, col_desc, col_desc_mut, compute_data_offset, header, header_mut, w32,
    ChunkHeader, ChunkState, Header, CHUNK_HEADER_SIZE, MAGIC, VERSION,
};
use crate::schema::{DType, Schema};
use crate::value::Value;
use std::mem;
use std::sync::atomic::Ordering;

pub(crate) fn append_row_unlocked(buf: &mut [u8], values: &[Value]) -> bool {
    assert!(
        validate_row_schema(buf, values),
        "value types do not match schema"
    );
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

    let row_data: usize = values.iter().map(|v| v.encoded_size()).sum();
    let total = 4 + row_data;
    if CHUNK_HEADER_SIZE + used + total > csz {
        return false;
    }

    let row_start = cs + CHUNK_HEADER_SIZE + used;
    w32(buf, row_start, row_data as u32);
    let mut off = row_start + 4;
    for v in values {
        v.encode(&mut buf[off..]);
        off += v.encoded_size();
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
        new_ch.generation.fetch_add(1, Ordering::Relaxed);
        new_ch.used.store(0, Ordering::Relaxed);
        new_ch.row_count.store(0, Ordering::Relaxed);
        new_ch
            .state
            .store(ChunkState::Writing as u32, Ordering::Relaxed);

        (&*(ptr as *const Header))
            .write_chunk
            .store(new_wc, Ordering::Release);
    }
}
/// Structural validation of a MemTable buffer.
///
/// Checks magic, version, layout offsets, column dtypes, and chunk states.
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
    for i in 0..h.num_chunks as usize {
        let cs = expected_off + i * csz;
        let state = chunk_header(buf, cs).state.load(Ordering::Relaxed);
        if state > 2 {
            return Err("invalid chunk state");
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
        let dt = DType::from_u32(col_desc(buf, i).dtype);
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
    h.num_cols = nc as u32;
    h.num_chunks = num_chunks;
    h.write_chunk.store(0, Ordering::Relaxed);
    h.data_offset = data_off as u32;
    h.chunk_size = chunk_size;
    h.write_lock.store(0, Ordering::Relaxed);
    h.refcount.store(1, Ordering::Relaxed);

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
