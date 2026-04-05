use crate::dedup::DedupState;
use crate::layout::{
    acquire_write_lock, chunk_header, chunk_start_off, col_desc, compute_data_offset, header,
    header_mut, release_write_lock, w32, CHUNK_HEADER_SIZE, FLAG_DEDUP,
};
use crate::raw::{
    advance_chunk_unlocked, init_buf, validate_buf, validate_row_schema, write_row_bytes,
};
use crate::refcount::refcount;
use crate::row::RowIter;
use crate::schema::{Col, DType, Schema, Value};
use crate::writer::RowWriter;
use std::fmt;
use std::sync::atomic::Ordering;

// ── Shared read-only accessor methods (expands inside each impl) ─────

macro_rules! impl_table_reader {
    () => {
        pub fn num_cols(&self) -> usize {
            header(self.as_bytes()).num_cols as usize
        }
        pub fn num_chunks(&self) -> usize {
            header(self.as_bytes()).num_chunks as usize
        }
        pub fn write_chunk(&self) -> usize {
            header(self.as_bytes()).write_chunk.load(Ordering::Acquire) as usize
        }
        pub fn data_offset(&self) -> usize {
            header(self.as_bytes()).data_offset as usize
        }
        pub fn chunk_size(&self) -> usize {
            header(self.as_bytes()).chunk_size as usize
        }
        pub fn refcount(&self) -> u32 {
            refcount(self.as_bytes())
        }
        pub fn col_name(&self, i: usize) -> &str {
            col_desc(self.as_bytes(), i).name_str()
        }
        pub fn col_dtype(&self, i: usize) -> Option<DType> {
            DType::from_u32(col_desc(self.as_bytes(), i).dtype)
        }
        pub fn col_elem_size(&self, i: usize) -> usize {
            col_desc(self.as_bytes(), i).elem_size as usize
        }
        pub fn chunk_used(&self, chunk: usize) -> usize {
            let buf = self.as_bytes();
            let cs = chunk_start_off(buf, chunk);
            chunk_header(buf, cs).used.load(Ordering::Acquire) as usize
        }
        pub fn chunk_generation(&self, chunk: usize) -> u64 {
            let buf = self.as_bytes();
            let cs = chunk_start_off(buf, chunk);
            chunk_header(buf, cs).generation.load(Ordering::Acquire)
        }
        pub fn chunk_state(&self, chunk: usize) -> u32 {
            let buf = self.as_bytes();
            let cs = chunk_start_off(buf, chunk);
            chunk_header(buf, cs).state.load(Ordering::Acquire)
        }
        pub fn rows(&self, chunk: usize) -> RowIter<'_> {
            let buf = self.as_bytes();
            let cs = chunk_start_off(buf, chunk);
            let ch = chunk_header(buf, cs);
            RowIter {
                buf,
                chunk_start: cs,
                pos: cs + CHUNK_HEADER_SIZE,
                end: cs + CHUNK_HEADER_SIZE + ch.used.load(Ordering::Acquire) as usize,
                generation: ch.generation.load(Ordering::Acquire),
            }
        }
        pub fn num_rows(&self, chunk: usize) -> usize {
            let buf = self.as_bytes();
            let cs = chunk_start_off(buf, chunk);
            chunk_header(buf, cs).row_count.load(Ordering::Acquire) as usize
        }
        pub fn creator_pid(&self) -> u32 {
            header(self.as_bytes()).creator_pid
        }
        pub fn creator_start_time(&self) -> u64 {
            header(self.as_bytes()).creator_start_time
        }
        pub fn schema(&self) -> Schema {
            let buf = self.as_bytes();
            let nc = header(buf).num_cols as usize;
            let mut s = Schema::new();
            for i in 0..nc {
                let cd = col_desc(buf, i);
                if let Some(dtype) = DType::from_u32(cd.dtype) {
                    s.cols.push(Col {
                        name: cd.name_str().to_string(),
                        dtype,
                        elem_size: cd.elem_size as usize,
                    });
                }
            }
            s
        }
    };
}

// ── Write helpers ────────────────────────────────────────────────────

fn make_row_writer<'a>(
    buf: &'a mut [u8],
    dedup: Option<&'a mut DedupState>,
    locked: bool,
) -> RowWriter<'a> {
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
        locked,
    }
}

fn begin_row_writer<'a>(buf: &'a mut [u8], dedup: Option<&'a mut DedupState>) -> RowWriter<'a> {
    acquire_write_lock(buf);
    make_row_writer(buf, dedup, true)
}

fn row_data_size(values: &[Value]) -> usize {
    values.iter().map(|v| v.encoded_size()).sum()
}

fn push_plain_row(buf: &mut [u8], values: &[Value]) {
    let row_data = row_data_size(values);
    if !write_row_bytes(buf, values, row_data) {
        advance_chunk_unlocked(buf);
        assert!(
            write_row_bytes(buf, values, row_data),
            "row exceeds chunk capacity"
        );
    }
}

fn locked_append(buf: &mut [u8], values: &[Value]) -> bool {
    acquire_write_lock(buf);
    let ok = write_row_bytes(buf, values, row_data_size(values));
    release_write_lock(buf);
    ok
}

fn locked_push(buf: &mut [u8], values: &[Value]) {
    acquire_write_lock(buf);
    push_plain_row(buf, values);
    release_write_lock(buf);
}

fn locked_advance(buf: &mut [u8]) {
    acquire_write_lock(buf);
    advance_chunk_unlocked(buf);
    release_write_lock(buf);
}

const MAX_DEDUP_COLS: usize = 64;

fn append_row_dedup_bytes(buf: &mut [u8], state: &mut DedupState, values: &[Value]) -> bool {
    debug_assert!(
        validate_row_schema(buf, values),
        "value types do not match schema"
    );

    let n = values.len();
    assert!(n <= MAX_DEDUP_COLS, "column count exceeds MAX_COLS");

    let h = header(buf);
    let wc = h.write_chunk.load(Ordering::Relaxed) as usize;
    let csz = h.chunk_size as usize;
    let cs = h.data_offset as usize + wc * csz;
    let used = chunk_header(buf, cs).used.load(Ordering::Relaxed) as usize;

    let mut lookups = [None::<usize>; MAX_DEDUP_COLS];
    let mut row_data = 0usize;
    for (i, v) in values.iter().enumerate() {
        let dup = match v {
            Value::Str(s) => state.lookup(i, s.as_bytes()),
            Value::Bytes(b) => state.lookup(i, b),
            _ => None,
        };
        lookups[i] = dup;
        row_data += if dup.is_some() { 4 } else { v.encoded_size() };
    }

    let total = 4 + row_data;
    if CHUNK_HEADER_SIZE + used + total > csz {
        return false;
    }

    let row_start = cs + CHUNK_HEADER_SIZE + used;
    w32(buf, row_start, row_data as u32);
    let mut off = row_start + 4;
    for (i, v) in values.iter().enumerate() {
        let var_data = match v {
            Value::Str(s) => Some(s.as_bytes()),
            Value::Bytes(b) => Some(*b),
            _ => None,
        };
        if let Some(data) = var_data {
            if let Some(ref_off) = lookups[i] {
                buf[off..off + 4].copy_from_slice(&(-(ref_off as i32)).to_le_bytes());
                off += 4;
            } else {
                let chunk_off = off - cs;
                let n = v.encode(&mut buf[off..]);
                state.insert(i, data, chunk_off);
                off += n;
            }
        } else {
            off += v.encode(&mut buf[off..]);
        }
    }
    chunk_header(buf, cs)
        .used
        .store((used + total) as u32, Ordering::Release);
    chunk_header(buf, cs)
        .row_count
        .fetch_add(1, Ordering::Release);
    true
}

// ── MemTable (owned buffer) ──────────────────────────────────────────

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

    pub fn from_buf(buf: Vec<u8>) -> Result<Self, &'static str> {
        validate_buf(&buf)?;
        Ok(Self { buf })
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.buf
    }
    pub fn view(&self) -> MemTableView<'_> {
        MemTableView { buf: &self.buf }
    }

    impl_table_reader!();

    pub fn row_writer(&mut self) -> RowWriter<'_> {
        begin_row_writer(&mut self.buf, None)
    }
    pub fn append_row(&mut self, values: &[Value]) -> bool {
        assert!(
            validate_row_schema(&self.buf, values),
            "value types do not match schema"
        );
        locked_append(&mut self.buf, values)
    }
    pub fn advance_chunk(&mut self) {
        locked_advance(&mut self.buf)
    }
    pub fn push_row(&mut self, values: &[Value]) {
        assert!(
            validate_row_schema(&self.buf, values),
            "value types do not match schema"
        );
        locked_push(&mut self.buf, values);
    }
    pub fn push_row_unchecked(&mut self, values: &[Value]) {
        locked_push(&mut self.buf, values);
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

// ── MemTableView (borrowed, read-only) ───────────────────────────────

pub struct MemTableView<'a> {
    buf: &'a [u8],
}

impl<'a> MemTableView<'a> {
    pub fn new(buf: &'a [u8]) -> Result<Self, &'static str> {
        validate_buf(buf)?;
        Ok(Self { buf })
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.buf
    }

    impl_table_reader!();
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

// ── MemTableWriter (borrowed, configurable write modes) ──────────────

/// Unified writer for external buffers (`&mut [u8]`).
///
/// Supports four modes via builder methods:
///
/// | Mode | Construction |
/// |------|-------------|
/// | Locked, plain | `MemTableWriter::new(buf)?` |
/// | Locked, dedup | `MemTableWriter::new(buf)?.dedup()` |
/// | Solo, plain | `MemTableWriter::new(buf)?.solo()` |
/// | Solo, dedup | `MemTableWriter::new(buf)?.solo().dedup()` |
///
/// **Locked** (default): writers are serialized via a spinlock — safe for
/// multiple writer threads sharing the same buffer through raw pointers.
///
/// **Solo**: no spinlock — the `&mut [u8]` borrow guarantees exclusive
/// access at compile time.  Saves ~5 ns/row of CAS overhead.
///
/// **Dedup**: per-chunk, hash-based string/bytes dedup.  Repeated values
/// are stored as 4-byte back-references within the same chunk.
pub struct MemTableWriter<'a> {
    buf: &'a mut [u8],
    dedup: Option<DedupState>,
    locked: bool,
}

impl<'a> MemTableWriter<'a> {
    pub fn new(buf: &'a mut [u8]) -> Result<Self, &'static str> {
        validate_buf(buf)?;
        Ok(Self {
            buf,
            dedup: None,
            locked: true,
        })
    }

    pub fn init(buf: &'a mut [u8], schema: &Schema, chunk_size: u32, num_chunks: u32) -> Self {
        init_buf(buf, schema, chunk_size, num_chunks);
        Self {
            buf,
            dedup: None,
            locked: true,
        }
    }

    /// Enable per-chunk string/bytes dedup.  Sets `FLAG_DEDUP` in header.
    pub fn dedup(mut self) -> Self {
        header_mut(self.buf).flags |= FLAG_DEDUP;
        self.dedup = Some(DedupState::new());
        self
    }

    /// Disable the spinlock (single-producer mode).
    pub fn solo(mut self) -> Self {
        self.locked = false;
        self
    }

    pub fn set_min_dedup_len(&mut self, len: usize) {
        if let Some(ref mut s) = self.dedup {
            s.set_min_dedup_len(len);
        }
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.buf
    }
    pub fn view(&self) -> MemTableView<'_> {
        MemTableView { buf: self.buf }
    }

    impl_table_reader!();

    pub fn row_writer(&mut self) -> RowWriter<'_> {
        if self.locked {
            begin_row_writer(self.buf, self.dedup.as_mut())
        } else {
            make_row_writer(self.buf, self.dedup.as_mut(), false)
        }
    }

    pub fn push_row(&mut self, values: &[Value]) {
        assert!(
            validate_row_schema(self.buf, values),
            "value types do not match schema"
        );
        self.push_inner(values);
    }

    pub fn push_row_unchecked(&mut self, values: &[Value]) {
        self.push_inner(values);
    }

    pub fn advance_chunk(&mut self) {
        if self.locked {
            acquire_write_lock(self.buf);
        }
        advance_chunk_unlocked(self.buf);
        if let Some(ref mut s) = self.dedup {
            s.clear();
        }
        if self.locked {
            release_write_lock(self.buf);
        }
    }

    pub fn append_row(&mut self, values: &[Value]) -> bool {
        assert!(
            validate_row_schema(self.buf, values),
            "value types do not match schema"
        );
        if self.locked {
            acquire_write_lock(self.buf);
        }
        let ok = if let Some(ref mut state) = self.dedup {
            append_row_dedup_bytes(self.buf, state, values)
        } else {
            write_row_bytes(self.buf, values, row_data_size(values))
        };
        if self.locked {
            release_write_lock(self.buf);
        }
        ok
    }

    fn push_inner(&mut self, values: &[Value]) {
        if self.locked {
            acquire_write_lock(self.buf);
        }
        if let Some(ref mut state) = self.dedup {
            if !append_row_dedup_bytes(self.buf, state, values) {
                advance_chunk_unlocked(self.buf);
                state.clear();
                assert!(
                    append_row_dedup_bytes(self.buf, state, values),
                    "row exceeds chunk capacity"
                );
            }
        } else {
            push_plain_row(self.buf, values);
        }
        if self.locked {
            release_write_lock(self.buf);
        }
    }
}

impl fmt::Display for MemTableWriter<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mode = match (self.locked, self.dedup.is_some()) {
            (true, false) => "locked",
            (true, true) => "locked+dedup",
            (false, false) => "solo",
            (false, true) => "solo+dedup",
        };
        write!(
            f,
            "MemTableWriter({} cols, {} chunks × {} bytes, {mode})",
            self.num_cols(),
            self.num_chunks(),
            self.chunk_size()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{MemTable, MemTableView, MemTableWriter};
    use crate::layout::{col_desc, header, header_mut, MAGIC, VERSION};
    use crate::raw::init_buf;
    use crate::refcount::{acquire_ref, refcount, release_ref};
    use crate::schema::{DType, Schema, Value};
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
        assert_eq!(t.col_dtype(0), Some(DType::I64));
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
        let mut w = MemTableWriter::init(&mut buf, &schema, 4096, 2);
        w.push_row(&[Value::I64(42), Value::Str("ext_test")]);
        let row = w.rows(0).next().unwrap();
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
        use crate::layout::{BYTE_ORDER_MARK, FLAGS_KNOWN};
        let schema = Schema::new().col("x", DType::I32);
        let t = MemTable::new(&schema, 1024, 4);
        let h = header(t.as_bytes());
        assert_eq!(h.magic, MAGIC);
        assert_eq!(h.version, VERSION);
        assert_eq!(
            h.header_size as usize,
            std::mem::size_of::<crate::layout::Header>()
        );
        assert_eq!(h.byte_order, u16::from_ne_bytes(BYTE_ORDER_MARK));
        assert_eq!(h.flags & !FLAGS_KNOWN, 0);
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
        assert!(MemTableView::new(&buf).is_err());
        assert!(MemTable::from_buf(buf).is_err());
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

            let mut producer = MemTableWriter::new(buf).unwrap();
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
            let mut mt = MemTableWriter::new(buf).unwrap();
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
            let mut mt = MemTableWriter::new(buf).unwrap();
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
                let mut mt = MemTableWriter::new(buf).unwrap();
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
            let mut mt = MemTableWriter::new(buf).unwrap();
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
        assert_eq!(t.chunk_state(0), 1);
        assert_eq!(t.chunk_generation(0), 1);
        assert_eq!(t.num_rows(0), 0);
        // Other chunks = Empty, generation 0
        assert_eq!(t.chunk_state(1), 0);
        assert_eq!(t.chunk_generation(1), 0);
    }

    #[test]
    fn chunk_state_transitions() {
        let schema = Schema::new().col("v", DType::I32);
        let mut t = MemTable::new(&schema, 1024, 3);

        // Write some rows to chunk 0
        t.push_row(&[Value::I32(1)]);
        t.push_row(&[Value::I32(2)]);
        assert_eq!(t.chunk_state(0), 1);
        assert_eq!(t.num_rows(0), 2);

        // Advance: chunk 0 → Sealed, chunk 1 → Writing
        t.advance_chunk();
        assert_eq!(t.chunk_state(0), 2);
        assert_eq!(t.chunk_state(1), 1);
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
        assert_eq!(t.chunk_state(0), 1);
    }

    #[test]
    fn num_rows_matches_iteration() {
        let schema = Schema::new().col("id", DType::I64).col("msg", DType::Str);
        let mut t = MemTable::new(&schema, 4096, 2);
        for i in 0..20i64 {
            t.push_row(&[Value::I64(i), Value::Str("hello")]);
        }
        assert_eq!(t.num_rows(0), t.rows(0).count());
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
            let mut dw = MemTableWriter::init(&mut buf_dedup, &schema, 65536, 1).dedup();
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
            let mut mt = MemTableWriter::init(&mut buf_plain, &schema, 65536, 1);
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
        let t = MemTable::new(&schema, 256, 2);
        let mut raw = t.as_bytes().to_vec();
        header_mut(&mut raw).version = 99;
        assert!(MemTable::from_buf(raw).is_err());
    }

    #[test]
    fn from_buf_rejects_bad_data_offset() {
        let schema = Schema::new().col("x", DType::U32);
        let t = MemTable::new(&schema, 256, 2);
        let mut raw = t.as_bytes().to_vec();
        header_mut(&mut raw).data_offset = 7;
        assert!(MemTable::from_buf(raw).is_err());
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

    // ── MemTableWriter solo mode tests ──────────────────────────

    #[test]
    fn solo_writer_basic() {
        let schema = Schema::new().col("ts", DType::I64).col("val", DType::F64);
        let size = MemTable::required_size(&schema, 4096, 2);
        let mut buf = vec![0u8; size];
        let mut sw = MemTableWriter::init(&mut buf, &schema, 4096, 2).solo();

        sw.push_row(&[Value::I64(100), Value::F64(3.14)]);
        sw.push_row(&[Value::I64(200), Value::F64(2.72)]);

        assert_eq!(sw.num_rows(0), 2);
        let mut rows = sw.rows(0);
        let mut c = rows.next().unwrap().cursor();
        assert_eq!(c.next_i64(), 100);
        assert_eq!(c.next_f64(), 3.14);
    }

    #[test]
    fn solo_writer_row_writer() {
        let schema = Schema::new().col("id", DType::I32).col("msg", DType::Str);
        let size = MemTable::required_size(&schema, 4096, 1);
        let mut buf = vec![0u8; size];
        let mut sw = MemTableWriter::init(&mut buf, &schema, 4096, 1).solo();

        sw.row_writer().put_i32(1).put_str("hello").finish();
        sw.row_writer().put_i32(2).put_str("world").finish();

        let rows: Vec<_> = sw.rows(0).collect();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].col_i32(0), 1);
        assert_eq!(rows[0].col_str(1), "hello");
        assert_eq!(rows[1].col_str(1), "world");
    }

    #[test]
    fn solo_writer_no_lock_touched() {
        let schema = Schema::new().col("x", DType::I32);
        let size = MemTable::required_size(&schema, 1024, 1);
        let mut buf = vec![0u8; size];
        let mut sw = MemTableWriter::init(&mut buf, &schema, 1024, 1).solo();
        sw.push_row(&[Value::I32(42)]);
        sw.row_writer().put_i32(99).finish();
        assert_eq!(
            header(sw.as_bytes()).write_lock.load(Ordering::Relaxed),
            0,
            "solo mode must never touch the write_lock"
        );
    }

    #[test]
    fn solo_writer_dedup() {
        let schema = Schema::new().col("tag", DType::Str).col("seq", DType::I32);
        let size = MemTable::required_size(&schema, 8192, 1);
        let mut buf = vec![0u8; size];
        let mut sw = MemTableWriter::init(&mut buf, &schema, 8192, 1)
            .solo()
            .dedup();

        for i in 0..20 {
            sw.push_row(&[Value::Str("repeat"), Value::I32(i)]);
        }

        let used_dedup = sw.chunk_used(0);

        // Compare with plain solo writer
        let mut buf2 = vec![0u8; size];
        let mut sw2 = MemTableWriter::init(&mut buf2, &schema, 8192, 1).solo();
        for i in 0..20 {
            sw2.push_row(&[Value::Str("repeat"), Value::I32(i)]);
        }
        let used_plain = sw2.chunk_used(0);

        assert!(
            used_dedup < used_plain,
            "dedup should save: {used_dedup} vs {used_plain}"
        );

        for (i, row) in sw.rows(0).enumerate() {
            let mut c = row.cursor();
            assert_eq!(c.next_str(), "repeat");
            assert_eq!(c.next_i32(), i as i32);
        }
    }

    #[test]
    fn solo_writer_auto_advance() {
        let schema = Schema::new().col("v", DType::I64);
        let size = MemTable::required_size(&schema, 64, 4);
        let mut buf = vec![0u8; size];
        let mut sw = MemTableWriter::init(&mut buf, &schema, 64, 4).solo();

        for i in 0..50i64 {
            sw.push_row_unchecked(&[Value::I64(i)]);
        }

        let mut total = 0;
        for chunk in 0..sw.num_chunks() {
            total += sw.num_rows(chunk);
        }
        assert!(total > 0, "should have rows across chunks");
    }
}
