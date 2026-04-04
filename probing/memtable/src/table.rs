use crate::buf::{advance_chunk_unlocked, append_row_unlocked, validate_row_schema};
use crate::dedup::DedupState;
use crate::layout::{
    acquire_write_lock, chunk_header, chunk_start_off, col_desc, header, release_write_lock,
    ChunkState, CHUNK_HEADER_SIZE,
};
use crate::row::RowIter;
use crate::schema::{Col, DType, Schema};
use crate::value::Value;
use crate::writer::RowWriter;
use std::sync::atomic::Ordering;

pub(crate) fn begin_row_writer<'a>(
    buf: &'a mut [u8],
    dedup: Option<&'a mut DedupState>,
) -> RowWriter<'a> {
    acquire_write_lock(buf);
    let h = header(buf);
    let wc = h.write_chunk.load(Ordering::Relaxed) as usize;
    let csz = h.chunk_size as usize;
    let doff = h.data_offset as usize;
    let cs = doff + wc * csz;
    let used = chunk_header(buf, cs).used.load(Ordering::Relaxed) as usize;
    RowWriter {
        buf,
        dedup,
        chunk_start: cs,
        chunk_size: csz,
        row_start: cs + CHUNK_HEADER_SIZE + used,
        pos: cs + CHUNK_HEADER_SIZE + used + 4,
        overflow: false,
        done: false,
        col_idx: 0,
    }
}

pub(crate) fn mt_append_row(buf: &mut [u8], values: &[Value]) -> bool {
    assert!(
        validate_row_schema(buf, values),
        "value types do not match schema"
    );
    acquire_write_lock(buf);
    let result = append_row_unlocked(buf, values);
    release_write_lock(buf);
    result
}

pub(crate) fn mt_push_row(buf: &mut [u8], values: &[Value]) {
    assert!(
        validate_row_schema(buf, values),
        "value types do not match schema"
    );
    acquire_write_lock(buf);
    if !append_row_unlocked(buf, values) {
        advance_chunk_unlocked(buf);
        assert!(
            append_row_unlocked(buf, values),
            "row exceeds chunk capacity"
        );
    }
    release_write_lock(buf);
}

pub(crate) fn mt_advance_chunk(buf: &mut [u8]) {
    acquire_write_lock(buf);
    advance_chunk_unlocked(buf);
    release_write_lock(buf);
}

pub(crate) fn mt_num_cols(buf: &[u8]) -> usize {
    header(buf).num_cols as usize
}
pub(crate) fn mt_num_chunks(buf: &[u8]) -> usize {
    header(buf).num_chunks as usize
}
pub(crate) fn mt_write_chunk(buf: &[u8]) -> usize {
    header(buf).write_chunk.load(Ordering::Acquire) as usize
}
pub(crate) fn mt_data_offset(buf: &[u8]) -> usize {
    header(buf).data_offset as usize
}
pub(crate) fn mt_chunk_size(buf: &[u8]) -> usize {
    header(buf).chunk_size as usize
}
pub(crate) fn mt_col_name(buf: &[u8], i: usize) -> &str {
    col_desc(buf, i).name_str()
}
pub(crate) fn mt_col_dtype(buf: &[u8], i: usize) -> DType {
    DType::from_u32(col_desc(buf, i).dtype)
}
pub(crate) fn mt_col_elem_size(buf: &[u8], i: usize) -> usize {
    col_desc(buf, i).elem_size as usize
}
pub(crate) fn mt_chunk_used(buf: &[u8], chunk: usize) -> usize {
    let cs = chunk_start_off(buf, chunk);
    chunk_header(buf, cs).used.load(Ordering::Acquire) as usize
}
pub(crate) fn mt_chunk_generation(buf: &[u8], chunk: usize) -> u64 {
    let cs = chunk_start_off(buf, chunk);
    chunk_header(buf, cs).generation.load(Ordering::Acquire)
}
pub(crate) fn mt_chunk_state(buf: &[u8], chunk: usize) -> ChunkState {
    let cs = chunk_start_off(buf, chunk);
    ChunkState::from_u32(chunk_header(buf, cs).state.load(Ordering::Acquire))
}
pub(crate) fn mt_chunk_row_count(buf: &[u8], chunk: usize) -> usize {
    let cs = chunk_start_off(buf, chunk);
    chunk_header(buf, cs).row_count.load(Ordering::Acquire) as usize
}
pub(crate) fn mt_rows<'a>(buf: &'a [u8], chunk: usize) -> RowIter<'a> {
    let cs = chunk_start_off(buf, chunk);
    let ch = chunk_header(buf, cs);
    let generation = ch.generation.load(Ordering::Acquire);
    let used = ch.used.load(Ordering::Acquire) as usize;
    RowIter {
        buf,
        chunk_start: cs,
        pos: cs + CHUNK_HEADER_SIZE,
        end: cs + CHUNK_HEADER_SIZE + used,
        generation,
    }
}
pub(crate) fn mt_num_rows(buf: &[u8], chunk: usize) -> usize {
    mt_rows(buf, chunk).count()
}
pub(crate) fn mt_schema(buf: &[u8]) -> Schema {
    let mut s = Schema::new();
    for i in 0..mt_num_cols(buf) {
        s.cols.push(Col {
            name: mt_col_name(buf, i).to_string(),
            dtype: mt_col_dtype(buf, i),
            elem_size: mt_col_elem_size(buf, i),
        });
    }
    s
}
