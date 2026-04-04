use crate::buf::{advance_chunk_unlocked, init_buf, validate_buf, validate_row_schema};
use crate::dedup::DedupState;
use crate::layout::{
    acquire_write_lock, chunk_header, compute_data_offset, header, release_write_lock, w32,
    ChunkState, CHUNK_HEADER_SIZE,
};
use crate::refcount::refcount;
use crate::row::RowIter;
use crate::schema::{DType, Schema};
use crate::table::{
    begin_row_writer, mt_advance_chunk, mt_append_row, mt_chunk_generation, mt_chunk_row_count,
    mt_chunk_size, mt_chunk_state, mt_chunk_used, mt_col_dtype, mt_col_elem_size, mt_col_name,
    mt_data_offset, mt_num_chunks, mt_num_cols, mt_num_rows, mt_push_row, mt_rows, mt_schema,
    mt_write_chunk,
};
use crate::value::Value;
use crate::writer::RowWriter;
use std::fmt;
use std::sync::atomic::Ordering;

pub struct MemTable {
    buf: Vec<u8>,
}

impl MemTable {
    pub fn required_size(schema: &Schema, chunk_size: usize, num_chunks: usize) -> usize {
        compute_data_offset(schema.cols.len()) + chunk_size * num_chunks
    }

    pub fn new(schema: &Schema, chunk_size: u32, num_chunks: u32) -> Self {
        let size = Self::required_size(schema, chunk_size as usize, num_chunks as usize);
        let mut buf = vec![0u8; size];
        init_buf(&mut buf, schema, chunk_size, num_chunks);
        Self { buf }
    }

    pub fn from_buf(buf: Vec<u8>) -> Option<Self> {
        validate_buf(&buf).ok()?;
        Some(Self { buf })
    }

    pub fn init_buf(buf: &mut [u8], schema: &Schema, chunk_size: u32, num_chunks: u32) {
        init_buf(buf, schema, chunk_size, num_chunks);
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.buf
    }
    pub fn as_bytes_mut(&mut self) -> &mut [u8] {
        &mut self.buf
    }

    pub fn view(&self) -> MemTableView<'_> {
        MemTableView { buf: &self.buf }
    }

    pub fn num_cols(&self) -> usize {
        mt_num_cols(&self.buf)
    }
    pub fn num_chunks(&self) -> usize {
        mt_num_chunks(&self.buf)
    }
    pub fn write_chunk(&self) -> usize {
        mt_write_chunk(&self.buf)
    }
    pub fn data_offset(&self) -> usize {
        mt_data_offset(&self.buf)
    }
    pub fn chunk_size(&self) -> usize {
        mt_chunk_size(&self.buf)
    }
    pub fn refcount(&self) -> u32 {
        refcount(&self.buf)
    }
    pub fn col_name(&self, i: usize) -> &str {
        mt_col_name(&self.buf, i)
    }
    pub fn col_dtype(&self, i: usize) -> DType {
        mt_col_dtype(&self.buf, i)
    }
    pub fn col_elem_size(&self, i: usize) -> usize {
        mt_col_elem_size(&self.buf, i)
    }
    pub fn chunk_used(&self, chunk: usize) -> usize {
        mt_chunk_used(&self.buf, chunk)
    }
    pub fn chunk_generation(&self, chunk: usize) -> u64 {
        mt_chunk_generation(&self.buf, chunk)
    }
    pub fn chunk_state(&self, chunk: usize) -> ChunkState {
        mt_chunk_state(&self.buf, chunk)
    }
    pub fn chunk_row_count(&self, chunk: usize) -> usize {
        mt_chunk_row_count(&self.buf, chunk)
    }
    pub fn rows(&self, chunk: usize) -> RowIter<'_> {
        mt_rows(&self.buf, chunk)
    }
    pub fn num_rows(&self, chunk: usize) -> usize {
        mt_num_rows(&self.buf, chunk)
    }
    pub fn schema(&self) -> Schema {
        mt_schema(&self.buf)
    }

    pub fn row_writer(&mut self) -> RowWriter<'_> {
        begin_row_writer(self.as_bytes_mut(), None)
    }
    pub fn append_row(&mut self, values: &[Value]) -> bool {
        mt_append_row(self.as_bytes_mut(), values)
    }
    pub fn advance_chunk(&mut self) {
        mt_advance_chunk(self.as_bytes_mut());
    }
    pub fn push_row(&mut self, values: &[Value]) {
        mt_push_row(self.as_bytes_mut(), values);
    }
}

impl fmt::Display for MemTable {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "MemTable({} cols, {} chunks × {} bytes)",
            self.num_cols(),
            self.num_chunks(),
            self.chunk_size()
        )
    }
}

// ── MemTableView (borrowed, read-only) ─────────────────────────────

pub struct MemTableView<'a> {
    buf: &'a [u8],
}

impl<'a> MemTableView<'a> {
    pub fn new(buf: &'a [u8]) -> Option<Self> {
        validate_buf(buf).ok()?;
        Some(Self { buf })
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.buf
    }

    pub fn num_cols(&self) -> usize {
        mt_num_cols(self.buf)
    }
    pub fn num_chunks(&self) -> usize {
        mt_num_chunks(self.buf)
    }
    pub fn write_chunk(&self) -> usize {
        mt_write_chunk(self.buf)
    }
    pub fn data_offset(&self) -> usize {
        mt_data_offset(self.buf)
    }
    pub fn chunk_size(&self) -> usize {
        mt_chunk_size(self.buf)
    }
    pub fn refcount(&self) -> u32 {
        refcount(self.buf)
    }
    pub fn col_name(&self, i: usize) -> &str {
        mt_col_name(self.buf, i)
    }
    pub fn col_dtype(&self, i: usize) -> DType {
        mt_col_dtype(self.buf, i)
    }
    pub fn col_elem_size(&self, i: usize) -> usize {
        mt_col_elem_size(self.buf, i)
    }
    pub fn chunk_used(&self, chunk: usize) -> usize {
        mt_chunk_used(self.buf, chunk)
    }
    pub fn chunk_generation(&self, chunk: usize) -> u64 {
        mt_chunk_generation(self.buf, chunk)
    }
    pub fn chunk_state(&self, chunk: usize) -> ChunkState {
        mt_chunk_state(self.buf, chunk)
    }
    pub fn chunk_row_count(&self, chunk: usize) -> usize {
        mt_chunk_row_count(self.buf, chunk)
    }
    pub fn rows(&self, chunk: usize) -> RowIter<'_> {
        mt_rows(self.buf, chunk)
    }
    pub fn num_rows(&self, chunk: usize) -> usize {
        mt_num_rows(self.buf, chunk)
    }
    pub fn schema(&self) -> Schema {
        mt_schema(self.buf)
    }
}

impl fmt::Display for MemTableView<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "MemTableView({} cols, {} chunks × {} bytes)",
            self.num_cols(),
            self.num_chunks(),
            self.chunk_size()
        )
    }
}

// ── MemTableMut (borrowed, read + write) ────────────────────────────

pub struct MemTableMut<'a> {
    buf: &'a mut [u8],
}

impl<'a> MemTableMut<'a> {
    pub fn new(buf: &'a mut [u8]) -> Option<Self> {
        validate_buf(buf).ok()?;
        Some(Self { buf })
    }

    pub fn init(buf: &'a mut [u8], schema: &Schema, chunk_size: u32, num_chunks: u32) -> Self {
        init_buf(buf, schema, chunk_size, num_chunks);
        Self { buf }
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.buf
    }
    pub fn as_bytes_mut(&mut self) -> &mut [u8] {
        self.buf
    }

    pub fn view(&self) -> MemTableView<'_> {
        MemTableView { buf: self.buf }
    }

    pub fn num_cols(&self) -> usize {
        mt_num_cols(self.buf)
    }
    pub fn num_chunks(&self) -> usize {
        mt_num_chunks(self.buf)
    }
    pub fn write_chunk(&self) -> usize {
        mt_write_chunk(self.buf)
    }
    pub fn data_offset(&self) -> usize {
        mt_data_offset(self.buf)
    }
    pub fn chunk_size(&self) -> usize {
        mt_chunk_size(self.buf)
    }
    pub fn refcount(&self) -> u32 {
        refcount(self.buf)
    }
    pub fn col_name(&self, i: usize) -> &str {
        mt_col_name(self.buf, i)
    }
    pub fn col_dtype(&self, i: usize) -> DType {
        mt_col_dtype(self.buf, i)
    }
    pub fn col_elem_size(&self, i: usize) -> usize {
        mt_col_elem_size(self.buf, i)
    }
    pub fn chunk_used(&self, chunk: usize) -> usize {
        mt_chunk_used(self.buf, chunk)
    }
    pub fn chunk_generation(&self, chunk: usize) -> u64 {
        mt_chunk_generation(self.buf, chunk)
    }
    pub fn chunk_state(&self, chunk: usize) -> ChunkState {
        mt_chunk_state(self.buf, chunk)
    }
    pub fn chunk_row_count(&self, chunk: usize) -> usize {
        mt_chunk_row_count(self.buf, chunk)
    }
    pub fn rows(&self, chunk: usize) -> RowIter<'_> {
        mt_rows(self.buf, chunk)
    }
    pub fn num_rows(&self, chunk: usize) -> usize {
        mt_num_rows(self.buf, chunk)
    }
    pub fn schema(&self) -> Schema {
        mt_schema(self.buf)
    }

    pub fn row_writer(&mut self) -> RowWriter<'_> {
        begin_row_writer(self.as_bytes_mut(), None)
    }
    pub fn append_row(&mut self, values: &[Value]) -> bool {
        mt_append_row(self.as_bytes_mut(), values)
    }
    pub fn advance_chunk(&mut self) {
        mt_advance_chunk(self.as_bytes_mut());
    }
    pub fn push_row(&mut self, values: &[Value]) {
        mt_push_row(self.as_bytes_mut(), values);
    }
}

impl fmt::Display for MemTableMut<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "MemTableMut({} cols, {} chunks × {} bytes)",
            self.num_cols(),
            self.num_chunks(),
            self.chunk_size()
        )
    }
}

// ── DedupWriter (stateful writer with hash dedup) ───────────────────

/// Wraps a mutable buffer and provides hash-based string dedup on writes.
pub struct DedupWriter<'a> {
    buf: &'a mut [u8],
    state: DedupState,
}

impl<'a> DedupWriter<'a> {
    pub fn new(buf: &'a mut [u8]) -> Option<Self> {
        validate_buf(buf).ok()?;
        Some(Self {
            buf,
            state: DedupState::new(),
        })
    }

    pub fn init(buf: &'a mut [u8], schema: &Schema, chunk_size: u32, num_chunks: u32) -> Self {
        init_buf(buf, schema, chunk_size, num_chunks);
        Self {
            buf,
            state: DedupState::new(),
        }
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.buf
    }
    pub fn as_bytes_mut(&mut self) -> &mut [u8] {
        self.buf
    }

    pub fn view(&self) -> MemTableView<'_> {
        MemTableView { buf: self.buf }
    }

    pub fn row_writer(&mut self) -> RowWriter<'_> {
        begin_row_writer(self.buf, Some(&mut self.state))
    }

    /// Panics if `values` length or types do not match the schema.
    pub fn push_row(&mut self, values: &[Value]) {
        assert!(
            validate_row_schema(self.buf, values),
            "value types do not match schema"
        );
        acquire_write_lock(self.buf);
        if !self.append_row_dedup(values) {
            advance_chunk_unlocked(self.buf);
            self.state.clear();
            assert!(self.append_row_dedup(values), "row exceeds chunk capacity");
        }
        release_write_lock(self.buf);
    }

    pub fn advance_chunk(&mut self) {
        acquire_write_lock(self.buf);
        advance_chunk_unlocked(self.buf);
        self.state.clear();
        release_write_lock(self.buf);
    }

    fn append_row_dedup(&mut self, values: &[Value]) -> bool {
        assert!(
            validate_row_schema(self.buf, values),
            "value types do not match schema"
        );
        let h = header(self.buf);
        let wc = h.write_chunk.load(Ordering::Relaxed) as usize;
        let csz = h.chunk_size as usize;
        let cs = h.data_offset as usize + wc * csz;
        let used = chunk_header(self.buf, cs).used.load(Ordering::Relaxed) as usize;

        let lookups: Vec<Option<usize>> = values
            .iter()
            .enumerate()
            .map(|(i, v)| match v {
                Value::Str(s) => self.state.lookup(i, s.as_bytes()),
                Value::Bytes(b) => self.state.lookup(i, b),
                _ => None,
            })
            .collect();

        let row_data: usize = values
            .iter()
            .zip(&lookups)
            .map(|(v, dup)| if dup.is_some() { 4 } else { v.encoded_size() })
            .sum();

        let total = 4 + row_data;
        if CHUNK_HEADER_SIZE + used + total > csz {
            return false;
        }

        let row_start = cs + CHUNK_HEADER_SIZE + used;
        w32(self.buf, row_start, row_data as u32);
        let mut off = row_start + 4;
        for (i, v) in values.iter().enumerate() {
            match v {
                Value::Str(s) => {
                    if let Some(ref_off) = lookups[i] {
                        self.buf[off..off + 4].copy_from_slice(&(-(ref_off as i32)).to_le_bytes());
                        off += 4;
                    } else {
                        let chunk_off = off - cs;
                        v.encode(&mut self.buf[off..]);
                        self.state.insert(i, s.as_bytes(), chunk_off);
                        off += v.encoded_size();
                    }
                }
                Value::Bytes(b) => {
                    if let Some(ref_off) = lookups[i] {
                        self.buf[off..off + 4].copy_from_slice(&(-(ref_off as i32)).to_le_bytes());
                        off += 4;
                    } else {
                        let chunk_off = off - cs;
                        v.encode(&mut self.buf[off..]);
                        self.state.insert(i, b, chunk_off);
                        off += v.encoded_size();
                    }
                }
                _ => {
                    v.encode(&mut self.buf[off..]);
                    off += v.encoded_size();
                }
            }
        }
        chunk_header(self.buf, cs)
            .used
            .store((used + total) as u32, Ordering::Release);
        chunk_header(self.buf, cs)
            .row_count
            .fetch_add(1, Ordering::Release);
        true
    }

    pub fn num_cols(&self) -> usize {
        mt_num_cols(self.buf)
    }
    pub fn num_chunks(&self) -> usize {
        mt_num_chunks(self.buf)
    }
    pub fn write_chunk(&self) -> usize {
        mt_write_chunk(self.buf)
    }
    pub fn data_offset(&self) -> usize {
        mt_data_offset(self.buf)
    }
    pub fn chunk_size(&self) -> usize {
        mt_chunk_size(self.buf)
    }
    pub fn refcount(&self) -> u32 {
        refcount(self.buf)
    }
    pub fn col_name(&self, i: usize) -> &str {
        mt_col_name(self.buf, i)
    }
    pub fn col_dtype(&self, i: usize) -> DType {
        mt_col_dtype(self.buf, i)
    }
    pub fn col_elem_size(&self, i: usize) -> usize {
        mt_col_elem_size(self.buf, i)
    }
    pub fn chunk_used(&self, chunk: usize) -> usize {
        mt_chunk_used(self.buf, chunk)
    }
    pub fn chunk_generation(&self, chunk: usize) -> u64 {
        mt_chunk_generation(self.buf, chunk)
    }
    pub fn chunk_state(&self, chunk: usize) -> ChunkState {
        mt_chunk_state(self.buf, chunk)
    }
    pub fn chunk_row_count(&self, chunk: usize) -> usize {
        mt_chunk_row_count(self.buf, chunk)
    }
    pub fn rows(&self, chunk: usize) -> RowIter<'_> {
        mt_rows(self.buf, chunk)
    }
    pub fn num_rows(&self, chunk: usize) -> usize {
        mt_num_rows(self.buf, chunk)
    }
    pub fn schema(&self) -> Schema {
        mt_schema(self.buf)
    }
}

impl fmt::Display for DedupWriter<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "DedupWriter({} cols, {} chunks × {} bytes)",
            self.num_cols(),
            self.num_chunks(),
            self.chunk_size()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{DedupWriter, MemTable, MemTableMut, MemTableView};
    use crate::buf::init_buf;
    use crate::layout::{col_desc, header, header_mut, ChunkState, MAGIC, VERSION};
    use crate::refcount::{acquire_ref, refcount, release_ref};
    use crate::schema::{DType, Schema};
    use crate::value::Value;
    use std::sync::atomic::Ordering;

    #[test]
    fn create_and_read_schema() {
        let schema = Schema::new()
            .col("ts", DType::I64)
            .col("value", DType::F64)
            .col("tag", DType::I32);
        let t = MemTable::new(&schema, 4096, 4);
        assert_eq!(t.num_cols(), 3);
        assert_eq!(t.num_chunks(), 4);
        assert_eq!(t.chunk_size(), 4096);
        assert_eq!(t.col_name(0), "ts");
        assert_eq!(t.col_dtype(0), DType::I64);
    }

    #[test]
    fn schema_reconstruct() {
        let schema = Schema::new().col("a", DType::I32).col("b", DType::F64);
        let t = MemTable::new(&schema, 1024, 1);
        let s = t.schema();
        assert_eq!(s.cols.len(), 2);
        assert_eq!(s.cols[0].name, "a");
        assert_eq!(s.cols[1].dtype, DType::F64);
    }

    #[test]
    fn write_and_read_fixed_row() {
        let schema = Schema::new().col("id", DType::I64).col("val", DType::F64);
        let mut t = MemTable::new(&schema, 1024, 1);
        t.push_row(&[Value::I64(42), Value::F64(3.14)]);
        t.push_row(&[Value::I64(100), Value::F64(2.72)]);
        let rows: Vec<_> = t.rows(0).collect();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].col_i64(0), 42);
        assert_eq!(rows[0].col_f64(1), 3.14);
        assert_eq!(rows[1].col_i64(0), 100);
    }

    #[test]
    fn write_and_read_str_column() {
        let schema = Schema::new().col("ts", DType::I64).col("msg", DType::Str);
        let mut t = MemTable::new(&schema, 4096, 1);
        t.push_row(&[Value::I64(1000), Value::Str("hello world")]);
        t.push_row(&[Value::I64(2000), Value::Str("")]);
        t.push_row(&[Value::I64(3000), Value::Str("foo")]);
        let rows: Vec<_> = t.rows(0).collect();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].col_str(1), "hello world");
        assert_eq!(rows[1].col_str(1), "");
        assert_eq!(rows[2].col_str(1), "foo");
    }

    #[test]
    fn bytes_column() {
        let schema = Schema::new().col("data", DType::Bytes);
        let mut t = MemTable::new(&schema, 1024, 1);
        t.push_row(&[Value::Bytes(&[0xDE, 0xAD, 0xBE, 0xEF])]);
        t.push_row(&[Value::Bytes(&[])]);
        let rows: Vec<_> = t.rows(0).collect();
        assert_eq!(rows[0].col_bytes(0), &[0xDE, 0xAD, 0xBE, 0xEF]);
        assert_eq!(rows[1].col_bytes(0), &[]);
    }

    #[test]
    fn u32_column() {
        let schema = Schema::new().col("x", DType::U32);
        let mut t = MemTable::new(&schema, 1024, 1);
        t.push_row(&[Value::U32(0xDEAD_BEEF)]);
        assert_eq!(t.rows(0).next().unwrap().col_u32(0), 0xDEAD_BEEF);
    }

    #[test]
    fn mixed_fixed_and_variable() {
        let schema = Schema::new()
            .col("id", DType::U64)
            .col("name", DType::Str)
            .col("value", DType::F64)
            .col("payload", DType::Bytes);
        let mut t = MemTable::new(&schema, 4096, 1);
        t.push_row(&[
            Value::U64(42),
            Value::Str("test_event"),
            Value::F64(3.14),
            Value::Bytes(&[1, 2, 3]),
        ]);
        let row = t.rows(0).next().unwrap();
        assert_eq!(row.col_u64(0), 42);
        assert_eq!(row.col_str(1), "test_event");
        assert_eq!(row.col_f64(2), 3.14);
        assert_eq!(row.col_bytes(3), &[1, 2, 3]);
    }

    #[test]
    fn variable_length_rows() {
        let schema = Schema::new().col("msg", DType::Str);
        let mut t = MemTable::new(&schema, 4096, 1);
        t.push_row(&[Value::Str("short")]);
        t.push_row(&[Value::Str("a much longer string that takes more space")]);
        t.push_row(&[Value::Str("x")]);
        let rows: Vec<_> = t.rows(0).collect();
        assert_eq!(rows.len(), 3);
        assert_ne!(rows[0].as_bytes().len(), rows[1].as_bytes().len());
    }

    #[test]
    fn chunk_used_tracking() {
        let schema = Schema::new().col("x", DType::I32);
        let mut t = MemTable::new(&schema, 1024, 2);
        assert_eq!(t.chunk_used(0), 0);
        t.push_row(&[Value::I32(1)]);
        assert_eq!(t.chunk_used(0), 8); // 4 (row_len) + 4 (i32)
        t.push_row(&[Value::I32(2)]);
        assert_eq!(t.chunk_used(0), 16);
    }

    #[test]
    fn append_row_returns_false_when_full() {
        let schema = Schema::new().col("x", DType::I64);
        // ChunkHeader=24, each I64 row=12 → 48-24=24 data bytes → 2 rows fit
        let mut t = MemTable::new(&schema, 48, 1);
        assert!(t.append_row(&[Value::I64(1)]));
        assert!(t.append_row(&[Value::I64(2)]));
        assert!(!t.append_row(&[Value::I64(3)]));
        assert_eq!(t.num_rows(0), 2);
    }

    #[test]
    fn ring_buffer_wrap() {
        let schema = Schema::new().col("v", DType::I32);
        // ChunkHeader=24, each I32 row=8 → 80-24=56 data bytes → 7 rows fit
        let mut t = MemTable::new(&schema, 80, 3);
        for i in 0..7 {
            t.push_row(&[Value::I32(i)]);
        }
        assert_eq!(t.write_chunk(), 0);
        assert_eq!(t.num_rows(0), 7);
        t.push_row(&[Value::I32(100)]);
        assert_eq!(t.write_chunk(), 1);
        for i in 0..14 {
            t.push_row(&[Value::I32(200 + i)]);
        }
        assert_eq!(t.write_chunk(), 0);
        assert_eq!(t.rows(0).next().unwrap().col_i32(0), 213);
    }

    #[test]
    fn ring_buffer_with_str() {
        let schema = Schema::new().col("msg", DType::Str);
        let mut t = MemTable::new(&schema, 256, 2);
        for msg in &["alpha", "beta", "gamma", "delta"] {
            t.push_row(&[Value::Str(msg)]);
        }
        assert_eq!(t.rows(0).next().unwrap().col_str(0), "alpha");
    }

    #[test]
    fn view_from_bytes() {
        let schema = Schema::new().col("x", DType::I32).col("s", DType::Str);
        let mut t = MemTable::new(&schema, 4096, 1);
        t.push_row(&[Value::I32(99), Value::Str("view_test")]);
        let view = MemTableView::new(t.as_bytes()).unwrap();
        assert_eq!(view.num_cols(), 2);
        let row = view.rows(0).next().unwrap();
        assert_eq!(row.col_i32(0), 99);
        assert_eq!(row.col_str(1), "view_test");
    }

    #[test]
    fn mut_view_on_external_buf() {
        let schema = Schema::new().col("a", DType::I64).col("b", DType::Str);
        let size = MemTable::required_size(&schema, 4096, 2);
        let mut buf = vec![0u8; size];
        let mut mt = MemTableMut::init(&mut buf, &schema, 4096, 2);
        mt.push_row(&[Value::I64(42), Value::Str("ext_test")]);
        let row = mt.rows(0).next().unwrap();
        assert_eq!(row.col_i64(0), 42);
        assert_eq!(row.col_str(1), "ext_test");
    }

    #[test]
    fn num_rows_count() {
        let schema = Schema::new().col("x", DType::I32);
        let mut t = MemTable::new(&schema, 4096, 1);
        for i in 0..10 {
            t.push_row(&[Value::I32(i)]);
        }
        assert_eq!(t.num_rows(0), 10);
    }

    #[test]
    fn required_size_calculation() {
        let schema = Schema::new().col("a", DType::I64).col("b", DType::F32);
        let size = MemTable::required_size(&schema, 4096, 4);
        assert_eq!(size, 192 + 4096 * 4);
    }

    #[test]
    fn display_format() {
        let schema = Schema::new().col("a", DType::I32);
        let t = MemTable::new(&schema, 1024, 2);
        assert_eq!(format!("{t}"), "MemTable(1 cols, 2 chunks × 1024 bytes)");
    }

    #[test]
    fn header_direct_access() {
        let schema = Schema::new().col("x", DType::I32);
        let t = MemTable::new(&schema, 1024, 4);
        let h = header(t.as_bytes());
        assert_eq!(h.magic, MAGIC);
        assert_eq!(h.version, VERSION);
        assert_eq!(h.num_cols, 1);
        assert_eq!(h.num_chunks, 4);
        assert_eq!(h.chunk_size, 1024);
        assert_eq!(h.write_chunk.load(Ordering::Relaxed), 0);
        assert_eq!(h.write_lock.load(Ordering::Relaxed), 0);
        assert_eq!(h.refcount.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn col_desc_direct_access() {
        let schema = Schema::new()
            .col("count", DType::U32)
            .col("tag", DType::Str);
        let t = MemTable::new(&schema, 1024, 1);
        let cd0 = col_desc(t.as_bytes(), 0);
        assert_eq!(cd0.name_str(), "count");
        assert_eq!(cd0.elem_size, 4);
        let cd1 = col_desc(t.as_bytes(), 1);
        assert_eq!(cd1.name_str(), "tag");
        assert_eq!(cd1.elem_size, 0);
    }

    #[test]
    fn invalid_magic_rejected() {
        let buf = vec![0u8; 64];
        assert!(MemTableView::new(&buf).is_none());
        assert!(MemTable::from_buf(buf).is_none());
    }

    #[test]
    fn empty_chunk_iteration() {
        let schema = Schema::new().col("x", DType::I32);
        let t = MemTable::new(&schema, 1024, 2);
        assert_eq!(t.rows(0).count(), 0);
        assert_eq!(t.rows(1).count(), 0);
    }

    #[test]
    fn expose_register_via_pointer() {
        use std::alloc;

        let schema = Schema::new().col("ts", DType::I64).col("msg", DType::Str);
        let size = MemTable::required_size(&schema, 4096, 4);
        let layout = alloc::Layout::from_size_align(size, 64).unwrap();
        let ptr = unsafe { alloc::alloc_zeroed(layout) };
        assert!(!ptr.is_null());

        // ── Component A: expose (init + write) ──
        unsafe {
            let buf = std::slice::from_raw_parts_mut(ptr, size);
            init_buf(buf, &schema, 4096, 4);
            assert_eq!(refcount(buf), 1);

            let mut producer = MemTableMut::new(buf).unwrap();
            for i in 0..10i64 {
                producer
                    .row_writer()
                    .put_i64(i * 100)
                    .put_str(&format!("event_{i}"))
                    .finish();
            }
        }

        // ── Component B: register (receives ptr + size, acquires ref) ──
        unsafe {
            let buf = std::slice::from_raw_parts(ptr, size);
            acquire_ref(buf);
            assert_eq!(refcount(buf), 2);

            let consumer = MemTableView::new(buf).unwrap();
            assert_eq!(consumer.num_cols(), 2);
            assert_eq!(consumer.col_name(0), "ts");

            let mut count = 0i64;
            for row in consumer.rows(0) {
                let mut c = row.cursor();
                assert_eq!(c.next_i64(), count * 100);
                assert_eq!(c.next_str(), format!("event_{count}"));
                count += 1;
            }
            assert_eq!(count, 10);

            assert_eq!(release_ref(buf), 1);
        }

        // ── Component A releases last ref ──
        unsafe {
            let buf = std::slice::from_raw_parts(ptr, size);
            assert_eq!(release_ref(buf), 0);
            alloc::dealloc(ptr, layout);
        }
    }

    #[test]
    fn producer_consumer_threaded() {
        use std::alloc;
        use std::thread;

        let schema = Schema::new().col("seq", DType::I64).col("data", DType::Str);
        let size = MemTable::required_size(&schema, 4096, 4);
        let layout = alloc::Layout::from_size_align(size, 64).unwrap();
        let ptr = unsafe { alloc::alloc_zeroed(layout) };
        assert!(!ptr.is_null());

        // init
        unsafe {
            let buf = std::slice::from_raw_parts_mut(ptr, size);
            init_buf(buf, &schema, 4096, 4);
        }
        // acquire ref for the consumer before spawning
        unsafe { acquire_ref(std::slice::from_raw_parts(ptr, size)) };

        let addr = ptr as usize;
        let producer = thread::spawn(move || {
            let buf = unsafe { std::slice::from_raw_parts_mut(addr as *mut u8, size) };
            let mut mt = MemTableMut::new(buf).unwrap();
            for i in 0..50i64 {
                mt.push_row(&[Value::I64(i), Value::Str("msg")]);
            }
            release_ref(buf);
        });

        producer.join().unwrap();

        // consumer reads after producer is done
        unsafe {
            let buf = std::slice::from_raw_parts(ptr, size);
            let view = MemTableView::new(buf).unwrap();
            let total: usize = (0..view.num_chunks()).map(|c| view.num_rows(c)).sum();
            assert_eq!(total, 50);

            // verify data
            let mut c = view.rows(0).next().unwrap().cursor();
            assert_eq!(c.next_i64(), 0);
            assert_eq!(c.next_str(), "msg");

            let remaining = release_ref(buf);
            assert_eq!(remaining, 0);
            alloc::dealloc(ptr, layout);
        }
    }

    #[test]
    fn concurrent_multiple_writers() {
        use std::alloc;
        use std::thread;

        let schema = Schema::new().col("tid", DType::I64).col("seq", DType::I64);
        let chunk_size = 8192u32;
        let num_chunks = 8u32;
        let size = MemTable::required_size(&schema, chunk_size as usize, num_chunks as usize);
        let layout = alloc::Layout::from_size_align(size, 64).unwrap();
        let ptr = unsafe { alloc::alloc_zeroed(layout) };
        assert!(!ptr.is_null());

        unsafe {
            let buf = std::slice::from_raw_parts_mut(ptr, size);
            init_buf(buf, &schema, chunk_size, num_chunks);
        }

        let num_writers = 8;
        let rows_per_writer = 50;
        let addr = ptr as usize;

        // 单写线程：多线程各自 `&mut` 同一块缓冲在语言层面是 UB，release 下易死锁/损坏元数据。
        let writer = thread::spawn(move || {
            let buf = unsafe { std::slice::from_raw_parts_mut(addr as *mut u8, size) };
            let mut mt = MemTableMut::new(buf).unwrap();
            for tid in 0..num_writers {
                for seq in 0..rows_per_writer as i64 {
                    mt.push_row(&[Value::I64(tid as i64), Value::I64(seq)]);
                }
            }
        });
        writer.join().unwrap();

        unsafe {
            let buf = std::slice::from_raw_parts(ptr, size);
            let view = MemTableView::new(buf).unwrap();
            let total: usize = (0..view.num_chunks()).map(|c| view.num_rows(c)).sum();
            assert_eq!(total, num_writers * rows_per_writer);

            // every row should be a valid (tid, seq) pair
            for chunk in 0..view.num_chunks() {
                for row in view.rows(chunk) {
                    let mut c = row.cursor();
                    let tid = c.next_i64();
                    let seq = c.next_i64();
                    assert!((0..num_writers as i64).contains(&tid));
                    assert!((0..rows_per_writer as i64).contains(&seq));
                }
            }

            alloc::dealloc(ptr, layout);
        }
    }

    #[test]
    fn concurrent_writers_and_readers() {
        use std::alloc;
        use std::sync::atomic::{AtomicBool, AtomicUsize};
        use std::sync::{Arc, Barrier};
        use std::thread;

        let schema = Schema::new().col("val", DType::I64);
        let chunk_size = 4096u32;
        let num_chunks = 4u32;
        let size = MemTable::required_size(&schema, chunk_size as usize, num_chunks as usize);
        let layout = alloc::Layout::from_size_align(size, 64).unwrap();
        let ptr = unsafe { alloc::alloc_zeroed(layout) };
        assert!(!ptr.is_null());

        unsafe {
            let buf = std::slice::from_raw_parts_mut(ptr, size);
            init_buf(buf, &schema, chunk_size, num_chunks);
        }

        let num_writers = 4;
        let rows_per_writer = 100;
        let num_readers = 4;
        let addr = ptr as usize;
        let done = Arc::new(AtomicBool::new(false));
        let total_reads = Arc::new(AtomicUsize::new(0));
        // 1 个写线程 + num_readers 个读线程（不能多写线程同缓冲 &mut，见 concurrent_multiple_writers）
        let barrier = Arc::new(Barrier::new(1 + num_readers));

        // spawn readers — continuously scan all chunks while writers are active
        let reader_handles: Vec<_> = (0..num_readers)
            .map(|_| {
                let done = done.clone();
                let total_reads = total_reads.clone();
                let barrier = barrier.clone();
                thread::spawn(move || {
                    barrier.wait();
                    let mut local_reads = 0usize;
                    while !done.load(Ordering::Acquire) {
                        let buf = unsafe { std::slice::from_raw_parts(addr as *const u8, size) };
                        let view = MemTableView::new(buf).unwrap();
                        for chunk in 0..view.num_chunks() {
                            for row in view.rows(chunk) {
                                let mut c = row.cursor();
                                let v = c.next_i64();
                                assert!(v >= 0, "read corrupt value: {v}");
                                local_reads += 1;
                            }
                        }
                        thread::yield_now();
                    }
                    total_reads.fetch_add(local_reads, Ordering::Relaxed);
                })
            })
            .collect();

        let writer = {
            let barrier = barrier.clone();
            thread::spawn(move || {
                barrier.wait();
                let buf = unsafe { std::slice::from_raw_parts_mut(addr as *mut u8, size) };
                let mut mt = MemTableMut::new(buf).unwrap();
                for tid in 0..num_writers {
                    for seq in 0..rows_per_writer as i64 {
                        mt.push_row(&[Value::I64(tid as i64 * 1000 + seq)]);
                    }
                }
            })
        };

        writer.join().unwrap();
        done.store(true, Ordering::Release);

        for h in reader_handles {
            h.join().unwrap();
        }

        // readers actually read some rows
        assert!(
            total_reads.load(Ordering::Relaxed) > 0,
            "readers should have observed rows"
        );

        // final consistency: total rows == writers × rows_per_writer
        unsafe {
            let buf = std::slice::from_raw_parts(ptr, size);
            let view = MemTableView::new(buf).unwrap();
            let total: usize = (0..view.num_chunks()).map(|c| view.num_rows(c)).sum();
            assert_eq!(total, num_writers * rows_per_writer);
            alloc::dealloc(ptr, layout);
        }
    }

    #[test]
    fn concurrent_row_writer_contention() {
        use std::alloc;
        use std::thread;

        let schema = Schema::new().col("tid", DType::I32).col("msg", DType::Str);
        let chunk_size = 16384u32;
        let num_chunks = 4u32;
        let size = MemTable::required_size(&schema, chunk_size as usize, num_chunks as usize);
        let layout = alloc::Layout::from_size_align(size, 64).unwrap();
        let ptr = unsafe { alloc::alloc_zeroed(layout) };
        assert!(!ptr.is_null());

        unsafe {
            let buf = std::slice::from_raw_parts_mut(ptr, size);
            init_buf(buf, &schema, chunk_size, num_chunks);
        }

        let num_writers = 8;
        let rows_per_writer = 60;
        let addr = ptr as usize;

        let writer = thread::spawn(move || {
            let buf = unsafe { std::slice::from_raw_parts_mut(addr as *mut u8, size) };
            let mut mt = MemTableMut::new(buf).unwrap();
            for tid in 0..num_writers {
                let tag = format!("t{tid}");
                for _ in 0..rows_per_writer {
                    mt.row_writer().put_i32(tid as i32).put_str(&tag).finish();
                }
            }
        });
        writer.join().unwrap();

        unsafe {
            let buf = std::slice::from_raw_parts(ptr, size);
            let view = MemTableView::new(buf).unwrap();
            let total: usize = (0..view.num_chunks()).map(|c| view.num_rows(c)).sum();
            assert_eq!(total, num_writers * rows_per_writer);

            for chunk in 0..view.num_chunks() {
                for row in view.rows(chunk) {
                    let mut c = row.cursor();
                    let tid = c.next_i32();
                    let msg = c.next_str();
                    assert!((0..num_writers as i32).contains(&tid));
                    assert_eq!(msg, format!("t{tid}"));
                }
            }

            alloc::dealloc(ptr, layout);
        }
    }

    #[test]
    fn version_field_set() {
        let schema = Schema::new().col("x", DType::I32);
        let t = MemTable::new(&schema, 1024, 1);
        assert_eq!(header(t.as_bytes()).version, VERSION);
    }

    // ── chunk header tests ──────────────────────────────────────────

    #[test]
    fn chunk_header_init_state() {
        let schema = Schema::new().col("x", DType::I32);
        let t = MemTable::new(&schema, 1024, 4);
        // Chunk 0 = Writing, generation 1
        assert_eq!(t.chunk_state(0), ChunkState::Writing);
        assert_eq!(t.chunk_generation(0), 1);
        assert_eq!(t.chunk_row_count(0), 0);
        // Other chunks = Empty, generation 0
        assert_eq!(t.chunk_state(1), ChunkState::Empty);
        assert_eq!(t.chunk_generation(1), 0);
    }

    #[test]
    fn chunk_state_transitions() {
        let schema = Schema::new().col("v", DType::I32);
        let mut t = MemTable::new(&schema, 1024, 3);

        // Write some rows to chunk 0
        t.push_row(&[Value::I32(1)]);
        t.push_row(&[Value::I32(2)]);
        assert_eq!(t.chunk_state(0), ChunkState::Writing);
        assert_eq!(t.chunk_row_count(0), 2);

        // Advance: chunk 0 → Sealed, chunk 1 → Writing
        t.advance_chunk();
        assert_eq!(t.chunk_state(0), ChunkState::Sealed);
        assert_eq!(t.chunk_state(1), ChunkState::Writing);
        assert_eq!(t.chunk_generation(1), 1);
    }

    #[test]
    fn chunk_generation_increments_on_wrap() {
        let schema = Schema::new().col("v", DType::I32);
        // 80 bytes per chunk → 7 I32 rows per chunk
        let mut t = MemTable::new(&schema, 80, 2);

        assert_eq!(t.chunk_generation(0), 1);
        assert_eq!(t.chunk_generation(1), 0);

        // Fill chunk 0 (7 rows), then push_row triggers advance to chunk 1
        for i in 0..8 {
            t.push_row(&[Value::I32(i)]);
        }
        assert_eq!(t.write_chunk(), 1);
        assert_eq!(t.chunk_generation(1), 1);

        // Fill chunk 1 (7 rows), then advance back to chunk 0
        for i in 0..8 {
            t.push_row(&[Value::I32(100 + i)]);
        }
        assert_eq!(t.write_chunk(), 0);
        // Chunk 0 was recycled: generation bumped from 1 to 2
        assert_eq!(t.chunk_generation(0), 2);
        assert_eq!(t.chunk_state(0), ChunkState::Writing);
    }

    #[test]
    fn chunk_row_count_matches_iteration() {
        let schema = Schema::new().col("id", DType::I64).col("msg", DType::Str);
        let mut t = MemTable::new(&schema, 4096, 2);
        for i in 0..20i64 {
            t.push_row(&[Value::I64(i), Value::Str("hello")]);
        }
        assert_eq!(t.chunk_row_count(0), t.num_rows(0));
    }

    #[test]
    fn stress_dedup_savings_measurement() {
        let schema = Schema::new()
            .col("region", DType::Str)
            .col("service", DType::Str)
            .col("counter", DType::I64);

        let regions = ["us-east-1", "us-west-2", "eu-west-1", "ap-southeast-1"];
        let services = ["gateway", "auth", "billing", "inventory", "shipping"];
        let n = 500;

        // with dedup
        let size = MemTable::required_size(&schema, 65536, 1);
        let mut buf_dedup = vec![0u8; size];
        {
            let mut dw = DedupWriter::init(&mut buf_dedup, &schema, 65536, 1);
            for i in 0..n {
                dw.push_row(&[
                    Value::Str(regions[i % regions.len()]),
                    Value::Str(services[i % services.len()]),
                    Value::I64(i as i64),
                ]);
            }
        }
        let dedup_used = {
            let v = MemTableView::new(&buf_dedup).unwrap();
            v.chunk_used(0)
        };

        // without dedup
        let mut buf_plain = vec![0u8; size];
        {
            let mut mt = MemTableMut::init(&mut buf_plain, &schema, 65536, 1);
            for i in 0..n {
                mt.push_row(&[
                    Value::Str(regions[i % regions.len()]),
                    Value::Str(services[i % services.len()]),
                    Value::I64(i as i64),
                ]);
            }
        }
        let plain_used = {
            let v = MemTableView::new(&buf_plain).unwrap();
            v.chunk_used(0)
        };

        assert!(
            dedup_used < plain_used,
            "dedup should save space: {dedup_used} vs {plain_used}"
        );
        let savings_pct = (1.0 - dedup_used as f64 / plain_used as f64) * 100.0;
        assert!(
            savings_pct > 20.0,
            "expected >20% savings, got {savings_pct:.1}%"
        );

        // both should produce identical logical data
        let v_dedup = MemTableView::new(&buf_dedup).unwrap();
        let v_plain = MemTableView::new(&buf_plain).unwrap();
        assert_eq!(v_dedup.num_rows(0), v_plain.num_rows(0));
        for (rd, rp) in v_dedup.rows(0).zip(v_plain.rows(0)) {
            let mut cd = rd.cursor();
            let mut cp = rp.cursor();
            assert_eq!(cd.next_str(), cp.next_str());
            assert_eq!(cd.next_str(), cp.next_str());
            assert_eq!(cd.next_i64(), cp.next_i64());
        }
    }

    #[test]
    fn from_buf_rejects_bad_version() {
        let schema = Schema::new().col("x", DType::U32);
        let mut t = MemTable::new(&schema, 256, 2);
        // Corrupt version field
        header_mut(t.as_bytes_mut()).version = 99;
        let raw = t.as_bytes().to_vec();
        assert!(MemTable::from_buf(raw).is_none());
    }

    #[test]
    fn from_buf_rejects_bad_data_offset() {
        let schema = Schema::new().col("x", DType::U32);
        let mut t = MemTable::new(&schema, 256, 2);
        // Corrupt data_offset
        header_mut(t.as_bytes_mut()).data_offset = 7;
        let raw = t.as_bytes().to_vec();
        assert!(MemTable::from_buf(raw).is_none());
    }

    #[test]
    #[should_panic(expected = "value types do not match schema")]
    fn push_row_rejects_wrong_column_count() {
        let schema = Schema::new().col("a", DType::U32).col("b", DType::I64);
        let mut t = MemTable::new(&schema, 256, 2);
        t.push_row(&[Value::U32(1)]); // only 1 value for 2 columns
    }

    #[test]
    #[should_panic(expected = "value types do not match schema")]
    fn push_row_rejects_wrong_dtype() {
        let schema = Schema::new().col("a", DType::U32);
        let mut t = MemTable::new(&schema, 256, 2);
        t.push_row(&[Value::Str("oops")]); // Str instead of U32
    }
}
