use crate::layout::{chunk_header, col_desc, r32};
use crate::schema::DType;
use std::sync::atomic::Ordering;

/// Unified panic for all stale-read conditions: the chunk was recycled while
/// data was being accessed, or the offset arithmetic fell outside the buffer.
#[cold]
#[inline(never)]
pub(crate) fn panic_stale(context: &str) -> ! {
    panic!("stale read: chunk recycled ({context})")
}

fn read_i32(data: &[u8], off: usize) -> i32 {
    if off + 4 > data.len() {
        panic_stale("read_i32");
    }
    i32::from_le_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]])
}

fn read_u32(data: &[u8], off: usize) -> u32 {
    if off + 4 > data.len() {
        panic_stale("read_u32");
    }
    u32::from_le_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]])
}

fn read_i64(data: &[u8], off: usize) -> i64 {
    if off + 8 > data.len() {
        panic_stale("read_i64");
    }
    i64::from_le_bytes([
        data[off],
        data[off + 1],
        data[off + 2],
        data[off + 3],
        data[off + 4],
        data[off + 5],
        data[off + 6],
        data[off + 7],
    ])
}

fn read_u64(data: &[u8], off: usize) -> u64 {
    if off + 8 > data.len() {
        panic_stale("read_u64");
    }
    u64::from_le_bytes([
        data[off],
        data[off + 1],
        data[off + 2],
        data[off + 3],
        data[off + 4],
        data[off + 5],
        data[off + 6],
        data[off + 7],
    ])
}

fn read_f32(data: &[u8], off: usize) -> f32 {
    if off + 4 > data.len() {
        panic_stale("read_f32");
    }
    f32::from_le_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]])
}

fn read_f64(data: &[u8], off: usize) -> f64 {
    if off + 8 > data.len() {
        panic_stale("read_f64");
    }
    f64::from_le_bytes([
        data[off],
        data[off + 1],
        data[off + 2],
        data[off + 3],
        data[off + 4],
        data[off + 5],
        data[off + 6],
        data[off + 7],
    ])
}

fn var_field_size(buf: &[u8], off: usize) -> usize {
    let raw = read_i32(buf, off);
    if raw < 0 {
        4
    } else {
        4 + raw as usize
    }
}

fn resolve_var(buf: &[u8], off: usize, chunk_start: usize) -> &[u8] {
    let raw = read_i32(buf, off);
    if raw < 0 {
        let ref_off = chunk_start + (-raw) as usize;
        if ref_off + 4 > buf.len() {
            panic_stale("resolve_var ref header");
        }
        let len = r32(buf, ref_off) as usize;
        let end = ref_off + 4 + len;
        if end > buf.len() {
            panic_stale("resolve_var ref payload");
        }
        &buf[ref_off + 4..end]
    } else {
        let len = raw as usize;
        let end = off + 4 + len;
        if end > buf.len() {
            panic_stale("resolve_var inline");
        }
        &buf[off + 4..end]
    }
}

// ── Row / RowIter ───────────────────────────────────────────────────

/// Read-only handle to a single row within a chunk.
///
/// Generation is validated once per row by [`RowIter::next()`], not on
/// every column access.  Call [`is_valid()`](Self::is_valid) explicitly
/// if you hold a `Row` across long-lived operations.
pub struct Row<'a> {
    pub(crate) data: &'a [u8],
    pub(crate) buf: &'a [u8],
    pub(crate) data_offset: usize,
    pub(crate) chunk_start: usize,
    pub(crate) generation: u64,
}

impl<'a> Row<'a> {
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Check whether the underlying chunk is still at the same generation.
    pub fn is_valid(&self) -> bool {
        chunk_header(self.buf, self.chunk_start)
            .generation
            .load(Ordering::Acquire)
            == self.generation
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.data
    }

    fn col_offset(&self, col: usize) -> usize {
        let mut off = 0;
        for i in 0..col {
            let dt = DType::from_u32(col_desc(self.buf, i).dtype)
                .unwrap_or_else(|| panic_stale("corrupt column dtype"));
            if let Some(sz) = dt.fixed_size() {
                off += sz;
            } else {
                off += var_field_size(self.data, off);
            }
        }
        off
    }

    fn resolve_var_col(&self, col: usize) -> &'a [u8] {
        let off = self.col_offset(col);
        resolve_var(self.buf, self.data_offset + off, self.chunk_start)
    }

    pub fn col_u8(&self, col: usize) -> u8 {
        if !self.is_valid() {
            return 0;
        }
        let off = self.col_offset(col);
        if off >= self.data.len() {
            return 0;
        }
        self.data[off]
    }
    pub fn col_u32(&self, col: usize) -> u32 {
        if !self.is_valid() {
            return 0;
        }
        read_u32(self.data, self.col_offset(col))
    }
    pub fn col_i32(&self, col: usize) -> i32 {
        if !self.is_valid() {
            return 0;
        }
        read_i32(self.data, self.col_offset(col))
    }
    pub fn col_i64(&self, col: usize) -> i64 {
        if !self.is_valid() {
            return 0;
        }
        read_i64(self.data, self.col_offset(col))
    }
    pub fn col_f32(&self, col: usize) -> f32 {
        if !self.is_valid() {
            return 0.0;
        }
        read_f32(self.data, self.col_offset(col))
    }
    pub fn col_f64(&self, col: usize) -> f64 {
        if !self.is_valid() {
            return 0.0;
        }
        read_f64(self.data, self.col_offset(col))
    }
    pub fn col_u64(&self, col: usize) -> u64 {
        if !self.is_valid() {
            return 0;
        }
        read_u64(self.data, self.col_offset(col))
    }
    pub fn col_str(&self, col: usize) -> &str {
        if !self.is_valid() {
            return "";
        }
        let b = self.resolve_var_col(col);
        if b.is_empty() {
            ""
        } else {
            std::str::from_utf8(b).unwrap_or("")
        }
    }
    pub fn col_bytes(&self, col: usize) -> &[u8] {
        if !self.is_valid() {
            return &[];
        }
        self.resolve_var_col(col)
    }

    pub fn cursor(&self) -> RowCursor<'a> {
        RowCursor {
            data: self.data,
            pos: 0,
            buf: self.buf,
            chunk_start: self.chunk_start,
            generation: self.generation,
            stale: !self.is_valid(),
        }
    }
}

/// Sequential cursor over columns within a row — O(1) per column.
///
/// Generation is validated once per row by [`RowIter::next()`].
/// If the chunk is recycled mid-read, the cursor is marked stale and
/// subsequent reads return zero / empty values instead of panicking.
pub struct RowCursor<'a> {
    data: &'a [u8],
    pos: usize,
    buf: &'a [u8],
    chunk_start: usize,
    generation: u64,
    stale: bool,
}

impl<'a> RowCursor<'a> {
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Check whether the underlying chunk is still at the same generation.
    pub fn is_valid(&self) -> bool {
        !self.stale
            && chunk_header(self.buf, self.chunk_start)
                .generation
                .load(Ordering::Acquire)
                == self.generation
    }

    /// Returns `true` if the chunk was recycled or a torn read was detected.
    pub fn is_stale(&self) -> bool {
        self.stale
    }

    fn mark_stale(&mut self) {
        self.stale = true;
    }

    fn read_fixed<const N: usize>(&mut self) -> [u8; N] {
        if self.stale {
            return [0u8; N];
        }
        let end = self.pos + N;
        if end > self.data.len() {
            self.mark_stale();
            return [0u8; N];
        }
        let mut v = [0u8; N];
        v.copy_from_slice(&self.data[self.pos..end]);
        self.pos += N;
        v
    }

    fn read_lp(&mut self) -> &'a [u8] {
        if self.stale {
            return &[];
        }
        let raw = i32::from_le_bytes(self.read_fixed::<4>());
        if self.stale {
            return &[];
        }
        if raw < 0 {
            let ref_off = self.chunk_start + (-raw) as usize;
            if ref_off + 4 > self.buf.len() {
                self.mark_stale();
                return &[];
            }
            let len = r32(self.buf, ref_off) as usize;
            let end = ref_off + 4 + len;
            if end > self.buf.len() {
                self.mark_stale();
                return &[];
            }
            &self.buf[ref_off + 4..end]
        } else {
            let len = raw as usize;
            if self.pos + len > self.data.len() {
                self.mark_stale();
                return &[];
            }
            let data = &self.data[self.pos..self.pos + len];
            self.pos += len;
            data
        }
    }

    pub fn next_u8(&mut self) -> u8 {
        self.read_fixed::<1>()[0]
    }
    pub fn next_u32(&mut self) -> u32 {
        u32::from_le_bytes(self.read_fixed())
    }
    pub fn next_i32(&mut self) -> i32 {
        i32::from_le_bytes(self.read_fixed())
    }
    pub fn next_i64(&mut self) -> i64 {
        i64::from_le_bytes(self.read_fixed())
    }
    pub fn next_f32(&mut self) -> f32 {
        f32::from_le_bytes(self.read_fixed())
    }
    pub fn next_f64(&mut self) -> f64 {
        f64::from_le_bytes(self.read_fixed())
    }
    pub fn next_u64(&mut self) -> u64 {
        u64::from_le_bytes(self.read_fixed())
    }
    pub fn next_str(&mut self) -> &'a str {
        let b = self.read_lp();
        if b.is_empty() {
            ""
        } else {
            std::str::from_utf8(b).unwrap_or("")
        }
    }
    pub fn next_bytes(&mut self) -> &'a [u8] {
        self.read_lp()
    }
}

/// Iterator over rows in a chunk.
///
/// Captures the chunk's `generation` at creation time.  Each call to
/// [`next()`](Iterator::next) checks generation **once**; if the chunk
/// was recycled it returns [`None`].  Column reads on the yielded [`Row`]
/// / [`RowCursor`] do **not** re-check, keeping the per-column path free
/// of atomic loads.
pub struct RowIter<'a> {
    pub(crate) buf: &'a [u8],
    pub(crate) chunk_start: usize,
    pub(crate) pos: usize,
    pub(crate) end: usize,
    pub(crate) generation: u64,
}

impl<'a> RowIter<'a> {
    /// The chunk generation captured when this iterator was created.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Returns `true` if the chunk's generation still matches the snapshot.
    /// A mismatch means the chunk was recycled and data may be stale.
    pub fn is_valid(&self) -> bool {
        chunk_header(self.buf, self.chunk_start)
            .generation
            .load(Ordering::Acquire)
            == self.generation
    }
}

impl<'a> Iterator for RowIter<'a> {
    type Item = Row<'a>;
    fn next(&mut self) -> Option<Self::Item> {
        while self.pos + 4 <= self.end {
            if !self.is_valid() {
                return None;
            }
            let row_len = r32(self.buf, self.pos) as usize;
            let row_total = 4usize.saturating_add(row_len);
            if row_total > self.end.saturating_sub(self.pos) {
                return None;
            }
            let row_end = self.pos + row_total;
            let data_offset = self.pos + 4;
            self.pos = row_end;
            if self.is_valid() {
                return Some(Row {
                    data: &self.buf[data_offset..row_end],
                    buf: self.buf,
                    data_offset,
                    chunk_start: self.chunk_start,
                    generation: self.generation,
                });
            }
            // Chunk recycled while parsing this row — skip it and continue.
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use crate::memtable::{MemTable, MemTableWriter};
    use crate::schema::{DType, Schema, Value};

    #[test]
    fn row_raw_bytes() {
        let schema = Schema::new().col("v", DType::I32);
        let mut t = MemTable::new(&schema, 1024, 1);
        t.push_row(&[Value::I32(0x12345678)]);
        assert_eq!(
            t.rows(0).next().unwrap().as_bytes(),
            &0x12345678_i32.to_le_bytes()
        );
    }

    #[test]
    fn row_cursor_basic() {
        let schema = Schema::new()
            .col("a", DType::I64)
            .col("b", DType::Str)
            .col("c", DType::F64)
            .col("d", DType::Bytes);
        let mut t = MemTable::new(&schema, 4096, 1);
        t.row_writer()
            .put_i64(42)
            .put_str("test")
            .put_f64(3.14)
            .put_bytes(&[1, 2, 3])
            .finish();
        let row = t.rows(0).next().unwrap();
        let mut c = row.cursor();
        assert_eq!(c.next_i64(), 42);
        assert_eq!(c.next_str(), "test");
        assert_eq!(c.next_f64(), 3.14);
        assert_eq!(c.next_bytes(), &[1, 2, 3]);
    }

    #[test]
    fn cursor_multiple_rows() {
        let schema = Schema::new().col("id", DType::I32).col("name", DType::Str);
        let mut t = MemTable::new(&schema, 4096, 1);
        for i in 0..5 {
            t.row_writer()
                .put_i32(i)
                .put_str(&format!("item_{i}"))
                .finish();
        }
        for (i, row) in t.rows(0).enumerate() {
            let mut c = row.cursor();
            assert_eq!(c.next_i32(), i as i32);
            assert_eq!(c.next_str(), format!("item_{i}"));
        }
    }
    #[test]
    fn row_iter_is_valid_detects_wrap() {
        let schema = Schema::new().col("v", DType::I32);
        let size = MemTable::required_size(&schema, 80, 2);
        let mut buf = vec![0u8; size];
        let mut mt = MemTableWriter::init(&mut buf, &schema, 80, 2);

        for i in 0..3 {
            mt.push_row(&[Value::I32(i)]);
        }

        // Capture generation of chunk 0
        let gen0 = mt.chunk_generation(0);

        // Advance twice: chunk 0 gets recycled
        mt.advance_chunk();
        mt.advance_chunk();

        // Generation changed → stale
        assert_ne!(mt.chunk_generation(0), gen0);
        assert_eq!(mt.chunk_generation(0), gen0 + 1);
    }
    #[test]
    fn row_becomes_invalid_after_wrap_and_col_asserts() {
        let schema = Schema::new().col("v", DType::I64);
        let mut t = MemTable::new(&schema, 80, 2);
        t.push_row(&[Value::I64(1)]);
        let row = t.rows(0).next().unwrap();
        let gen_before = row.generation();
        assert_eq!(row.col_i64(0), 1);

        // wrap chunk 0 twice: 0→1→0 so chunk 0 gets a new generation
        t.advance_chunk();
        t.advance_chunk();
        let gen_after = t.chunk_generation(0);
        assert_ne!(
            gen_before, gen_after,
            "generation should have changed after wrap"
        );
    }

    #[test]
    fn row_col_degrades_on_stale_chunk_without_panic() {
        use crate::memtable::MemTableView;

        let schema = Schema::new().col("v", DType::I64);
        let size = MemTable::required_size(&schema, 80, 2);
        let mut buf = vec![0u8; size];
        {
            let mut mt = MemTableWriter::init(&mut buf, &schema, 80, 2);
            mt.push_row(&[Value::I64(42)]);
        }
        let reader: &[u8] = unsafe { std::slice::from_raw_parts(buf.as_ptr(), buf.len()) };
        let view = MemTableView::new(reader).unwrap();
        let row = view.rows(0).next().unwrap();

        {
            let mut w = MemTableWriter::new(&mut buf).unwrap();
            w.advance_chunk();
            w.advance_chunk();
        }
        assert!(!row.is_valid());
        assert_eq!(row.col_i64(0), 0);
    }

    #[test]
    fn row_cursor_degrades_on_stale_chunk_without_panic() {
        use crate::memtable::MemTableView;

        let schema = Schema::new().col("v", DType::I64).col("tag", DType::Str);
        let size = MemTable::required_size(&schema, 80, 2);
        let mut buf = vec![0u8; size];
        {
            let mut mt = MemTableWriter::init(&mut buf, &schema, 80, 2);
            mt.push_row(&[Value::I64(42), Value::Str("hello")]);
        }
        let reader: &[u8] = unsafe { std::slice::from_raw_parts(buf.as_ptr(), buf.len()) };
        let view = MemTableView::new(reader).unwrap();
        let gen = view.chunk_generation(0);
        let row = view.rows(0).next().unwrap();

        {
            let mut w = MemTableWriter::new(&mut buf).unwrap();
            w.advance_chunk();
            w.advance_chunk();
        }
        assert_ne!(MemTableView::new(reader).unwrap().chunk_generation(0), gen);
        assert!(!row.is_valid());

        let mut c = row.cursor();
        assert!(c.is_stale());
        assert_eq!(c.next_i64(), 0);
        assert_eq!(c.next_str(), "");
    }

    #[test]
    fn row_iter_skips_row_when_chunk_recycled_mid_parse() {
        use crate::memtable::MemTableView;

        let schema = Schema::new().col("v", DType::I32);
        let size = MemTable::required_size(&schema, 80, 2);
        let mut buf = vec![0u8; size];
        {
            let mut mt = MemTableWriter::init(&mut buf, &schema, 80, 2);
            for i in 0..3 {
                mt.push_row(&[Value::I32(i)]);
            }
        }
        let reader: &[u8] = unsafe { std::slice::from_raw_parts(buf.as_ptr(), buf.len()) };
        let view = MemTableView::new(reader).unwrap();
        let gen = view.chunk_generation(0);
        let mut iter = view.rows(0);
        let _ = iter.next().unwrap();

        {
            let mut w = MemTableWriter::new(&mut buf).unwrap();
            w.advance_chunk();
            w.advance_chunk();
        }
        assert_ne!(MemTableView::new(reader).unwrap().chunk_generation(0), gen);
        assert!(iter.next().is_none());
    }
}
