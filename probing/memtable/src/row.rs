use crate::layout::{chunk_header, col_desc, r32};
use crate::schema::DType;
use std::sync::atomic::Ordering;

/// Encoded size of a variable-length field at `off`: 4 if reference (negative), else 4+len.
fn var_field_size(buf: &[u8], off: usize) -> usize {
    if off + 4 > buf.len() {
        panic!("stale Row: var_field_size out of bounds");
    }
    let raw = i32::from_le_bytes(buf[off..off + 4].try_into().unwrap());
    if raw < 0 {
        4
    } else {
        4 + raw as usize
    }
}

/// Resolve a variable-length field to its actual bytes.
/// If `raw` (i32 at `off`) is negative, follow the reference within the chunk.
fn resolve_var<'a>(buf: &'a [u8], off: usize, chunk_start: usize) -> &'a [u8] {
    if off + 4 > buf.len() {
        panic!("stale Row: resolve_var out of bounds");
    }
    let raw = i32::from_le_bytes(buf[off..off + 4].try_into().unwrap());
    if raw < 0 {
        let ref_off = chunk_start + (-raw) as usize;
        if ref_off + 4 > buf.len() {
            panic!("stale Row: resolve_var ref header out of bounds");
        }
        let len = r32(buf, ref_off) as usize;
        let end = ref_off + 4 + len;
        if end > buf.len() {
            panic!("stale Row: resolve_var ref payload out of bounds");
        }
        &buf[ref_off + 4..end]
    } else {
        let len = raw as usize;
        let end = off + 4 + len;
        if end > buf.len() {
            panic!("stale Row: resolve_var inline out of bounds");
        }
        &buf[off + 4..end]
    }
}

// ── Row / RowIter ───────────────────────────────────────────────────

/// Read-only handle to a single row within a chunk.
///
/// Carries the chunk's `generation` at creation time. Access checks that the
/// chunk generation still matches (release builds included), so stale reads
/// after ring-buffer wrap fail fast.
pub struct Row<'a> {
    pub(crate) data: &'a [u8],
    pub(crate) buf: &'a [u8],
    pub(crate) chunk_start: usize,
    pub(crate) generation: u64,
}

impl<'a> Row<'a> {
    pub fn generation(&self) -> u64 {
        self.generation
    }

    fn assert_generation(&self) {
        assert_eq!(
            chunk_header(self.buf, self.chunk_start)
                .generation
                .load(Ordering::Acquire),
            self.generation,
            "stale Row: chunk has been recycled"
        );
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.data
    }

    fn col_offset(&self, col: usize) -> usize {
        self.assert_generation();
        let mut off = 0;
        for i in 0..col {
            let dt = DType::from_u32(col_desc(self.buf, i).dtype);
            if let Some(sz) = dt.fixed_size() {
                off += sz;
            } else {
                off += var_field_size(self.data, off);
            }
        }
        off
    }

    pub fn col_u8(&self, col: usize) -> u8 {
        self.data[self.col_offset(col)]
    }
    pub fn col_u32(&self, col: usize) -> u32 {
        let off = self.col_offset(col);
        u32::from_le_bytes(self.data[off..off + 4].try_into().unwrap())
    }
    pub fn col_i32(&self, col: usize) -> i32 {
        let off = self.col_offset(col);
        i32::from_le_bytes(self.data[off..off + 4].try_into().unwrap())
    }
    pub fn col_i64(&self, col: usize) -> i64 {
        let off = self.col_offset(col);
        i64::from_le_bytes(self.data[off..off + 8].try_into().unwrap())
    }
    pub fn col_f32(&self, col: usize) -> f32 {
        let off = self.col_offset(col);
        f32::from_le_bytes(self.data[off..off + 4].try_into().unwrap())
    }
    pub fn col_f64(&self, col: usize) -> f64 {
        let off = self.col_offset(col);
        f64::from_le_bytes(self.data[off..off + 8].try_into().unwrap())
    }
    pub fn col_u64(&self, col: usize) -> u64 {
        let off = self.col_offset(col);
        u64::from_le_bytes(self.data[off..off + 8].try_into().unwrap())
    }
    pub fn col_str(&self, col: usize) -> &str {
        let off = self.col_offset(col);
        let b = resolve_var(
            self.buf,
            self.data.as_ptr() as usize - self.buf.as_ptr() as usize + off,
            self.chunk_start,
        );
        if b.is_empty() {
            ""
        } else {
            std::str::from_utf8(b).unwrap_or("")
        }
    }
    pub fn col_bytes(&self, col: usize) -> &[u8] {
        let off = self.col_offset(col);
        resolve_var(
            self.buf,
            self.data.as_ptr() as usize - self.buf.as_ptr() as usize + off,
            self.chunk_start,
        )
    }

    pub fn cursor(&self) -> RowCursor<'a> {
        self.assert_generation();
        RowCursor {
            data: self.data,
            pos: 0,
            buf: self.buf,
            chunk_start: self.chunk_start,
            generation: self.generation,
        }
    }
}

/// Sequential cursor over columns within a row — O(1) per column.
///
/// Like [`Row`], carries a chunk `generation` snapshot; each read checks it
/// so wrap-after-hold is caught in release builds too.
pub struct RowCursor<'a> {
    data: &'a [u8],
    pos: usize,
    buf: &'a [u8],
    chunk_start: usize,
    generation: u64,
}

impl<'a> RowCursor<'a> {
    pub fn generation(&self) -> u64 {
        self.generation
    }

    fn assert_generation(&self) {
        assert_eq!(
            chunk_header(self.buf, self.chunk_start)
                .generation
                .load(Ordering::Acquire),
            self.generation,
            "stale RowCursor: chunk has been recycled"
        );
    }

    fn read_fixed<const N: usize>(&mut self) -> [u8; N] {
        self.assert_generation();
        let v: [u8; N] = self.data[self.pos..self.pos + N].try_into().unwrap();
        self.pos += N;
        v
    }

    fn read_lp(&mut self) -> &'a [u8] {
        let raw = i32::from_le_bytes(self.read_fixed::<4>());
        if raw < 0 {
            let ref_off = self.chunk_start + (-raw) as usize;
            if ref_off + 4 > self.buf.len() {
                panic!("stale RowCursor: dedup ref out of bounds");
            }
            let len = r32(self.buf, ref_off) as usize;
            let end = ref_off + 4 + len;
            if end > self.buf.len() {
                panic!("stale RowCursor: dedup ref out of bounds");
            }
            &self.buf[ref_off + 4..end]
        } else {
            let len = raw as usize;
            if self.pos + len > self.data.len() {
                panic!("stale RowCursor: inline str out of bounds");
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
/// Captures the chunk's `generation` at creation time. Under concurrent writers,
/// `next()` returns [`None`] if the chunk was recycled or the row length does
/// not fit the iterator snapshot — no panic. After a row is yielded, column
/// access on [`Row`] / [`RowCursor`] still checks generation and may panic if
/// the chunk wrapped before you read (fail-fast for single-thread misuse too).
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
        if self.pos >= self.end {
            return None;
        }
        if !self.is_valid() {
            return None;
        }
        if self.pos + 4 > self.end {
            return None;
        }
        let row_len = r32(self.buf, self.pos) as usize;
        let row_total = 4usize.saturating_add(row_len);
        if row_total > self.end.saturating_sub(self.pos) {
            return None;
        }
        if !self.is_valid() {
            return None;
        }
        let row_end = self.pos + row_total;
        let data = &self.buf[self.pos + 4..row_end];
        self.pos = row_end;
        Some(Row {
            data,
            buf: self.buf,
            chunk_start: self.chunk_start,
            generation: self.generation,
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::memtable::{MemTable, MemTableMut};
    use crate::schema::{DType, Schema};
    use crate::value::Value;

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
        let mut mt = MemTableMut::init(&mut buf, &schema, 80, 2);

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
        let rows: Vec<_> = t.rows(0).collect();
        assert_eq!(rows[0].col_i64(0), 1);
        let gen_before = rows[0].generation();

        // wrap chunk 0 twice: 0→1→0 so chunk 0 gets a new generation
        t.advance_chunk();
        t.advance_chunk();
        let gen_after = t.chunk_generation(0);
        assert_ne!(
            gen_before, gen_after,
            "generation should have changed after wrap"
        );
    }
}
