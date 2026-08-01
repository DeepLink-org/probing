use crate::dedup::DedupState;
use crate::layout::{chunk_header, col_desc, w32, CHUNK_HEADER_SIZE};
use crate::raw::note_row_ts;
use crate::schema::DType;
use std::sync::atomic::Ordering;

/// Streaming row writer — low-overhead, schema-checked hot-path API.
///
/// MEMT is single-writer, so no lock is taken; the `&mut` borrow guarantees
/// exclusive access for the writer's lifetime.
///
/// Callers supply columns in schema order via the typed `put_*` methods.
/// A type/count mismatch makes [`finish`](Self::finish) return `false` and
/// commits nothing.
///
/// When created from a writer with dedup enabled, string/bytes columns
/// participate in hash-based dedup automatically.
pub struct RowWriter<'a> {
    pub(crate) buf: &'a mut [u8],
    pub(crate) dedup: Option<&'a mut DedupState>,
    pub(crate) chunk_start: usize,
    pub(crate) chunk_size: usize,
    pub(crate) row_start: usize,
    pub(crate) pos: usize,
    pub(crate) overflow: bool,
    pub(crate) done: bool,
    pub(crate) col_idx: usize,
    pub(crate) expected_cols: usize,
    pub(crate) schema_mismatch: bool,
    /// `Header::ts_col` (timestamp column index + 1; 0 = none).
    pub(crate) ts_col: u16,
    /// Timestamp captured by `put_i64` on the designated column,
    /// folded into the chunk's min/max on a successful `finish()`.
    pub(crate) pending_ts: Option<i64>,
}

impl<'a> RowWriter<'a> {
    fn take_column(&mut self, dtype: DType) -> bool {
        let matches = self.col_idx < self.expected_cols
            && col_desc(self.buf, self.col_idx).dtype == dtype as u32;
        self.col_idx += 1;
        self.schema_mismatch |= !matches;
        matches
    }

    fn can_write(&self, n: usize) -> bool {
        !self.overflow && self.pos + n <= self.chunk_start + self.chunk_size
    }

    fn write_raw(&mut self, bytes: &[u8]) {
        if self.can_write(bytes.len()) {
            self.buf[self.pos..self.pos + bytes.len()].copy_from_slice(bytes);
            self.pos += bytes.len();
        } else {
            self.overflow = true;
        }
    }

    fn write_lp(&mut self, data: &[u8]) {
        if self.can_write(4 + data.len()) {
            w32(self.buf, self.pos, data.len() as u32);
            self.buf[self.pos + 4..self.pos + 4 + data.len()].copy_from_slice(data);
            self.pos += 4 + data.len();
        } else {
            self.overflow = true;
        }
    }

    fn write_str_dedup(&mut self, data: &[u8]) {
        if !self.overflow {
            if let Some(dedup) = self.dedup.as_ref() {
                if let Some(off) = dedup.lookup(self.col_idx, data, self.buf, self.chunk_start) {
                    self.write_raw(&(-(off as i32)).to_le_bytes());
                    return;
                }
            }
        }
        let chunk_off = self.pos - self.chunk_start;
        self.write_lp(data);
        if !self.overflow {
            if let Some(dedup) = self.dedup.as_mut() {
                dedup.stage_insert(self.col_idx, data, chunk_off);
            }
        }
    }

    pub fn put_u8(&mut self, v: u8) -> &mut Self {
        if self.take_column(DType::U8) {
            self.write_raw(&[v]);
        }
        self
    }
    pub fn put_u32(&mut self, v: u32) -> &mut Self {
        if self.take_column(DType::U32) {
            self.write_raw(&v.to_le_bytes());
        }
        self
    }
    pub fn put_i32(&mut self, v: i32) -> &mut Self {
        if self.take_column(DType::I32) {
            self.write_raw(&v.to_le_bytes());
        }
        self
    }
    pub fn put_i64(&mut self, v: i64) -> &mut Self {
        let col_idx = self.col_idx;
        if self.take_column(DType::I64) {
            if self.ts_col as usize == col_idx + 1 {
                self.pending_ts = Some(v);
            }
            self.write_raw(&v.to_le_bytes());
        }
        self
    }
    pub fn put_f32(&mut self, v: f32) -> &mut Self {
        if self.take_column(DType::F32) {
            self.write_raw(&v.to_le_bytes());
        }
        self
    }
    pub fn put_f64(&mut self, v: f64) -> &mut Self {
        if self.take_column(DType::F64) {
            self.write_raw(&v.to_le_bytes());
        }
        self
    }
    pub fn put_u64(&mut self, v: u64) -> &mut Self {
        if self.take_column(DType::U64) {
            self.write_raw(&v.to_le_bytes());
        }
        self
    }

    pub fn put_str(&mut self, s: &str) -> &mut Self {
        if self.take_column(DType::Str) {
            if self.dedup.is_some() {
                self.write_str_dedup(s.as_bytes());
            } else {
                self.write_lp(s.as_bytes());
            }
        }
        self
    }
    pub fn put_bytes(&mut self, b: &[u8]) -> &mut Self {
        if self.take_column(DType::Bytes) {
            if self.dedup.is_some() {
                self.write_str_dedup(b);
            } else {
                self.write_lp(b);
            }
        }
        self
    }

    /// Commit the row. Returns `false` if the row overflowed the chunk (and
    /// nothing was committed) or `finish` was already called.
    pub fn finish(&mut self) -> bool {
        if self.done {
            return false;
        }
        self.done = true;
        let ok = if self.overflow || self.schema_mismatch || self.col_idx != self.expected_cols {
            if let Some(dedup) = self.dedup.as_mut() {
                dedup.rollback_row();
            }
            false
        } else {
            let row_data = self.pos - self.row_start - 4;
            w32(self.buf, self.row_start, row_data as u32);
            let new_used = (self.pos - self.chunk_start - CHUNK_HEADER_SIZE) as u32;
            if let Some(ts) = self.pending_ts {
                note_row_ts(chunk_header(self.buf, self.chunk_start), ts);
            }
            chunk_header(self.buf, self.chunk_start)
                .used
                .store(new_used, Ordering::Release);
            chunk_header(self.buf, self.chunk_start)
                .row_count
                .fetch_add(1, Ordering::Release);
            if let Some(dedup) = self.dedup.as_mut() {
                dedup.commit_row();
            }
            true
        };
        ok
    }
}

impl Drop for RowWriter<'_> {
    fn drop(&mut self) {
        if !self.done {
            if let Some(dedup) = self.dedup.as_mut() {
                dedup.rollback_row();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::memtable::{MemTable, MemTableWriter};
    use crate::schema::{DType, Schema, Value};

    #[test]
    fn row_writer_basic() {
        let schema = Schema::new().col("id", DType::I64).col("val", DType::F64);
        let mut t = MemTable::new(&schema, 1024, 1).unwrap();
        assert!(t.row_writer().put_i64(42).put_f64(3.14).finish());
        assert!(t.row_writer().put_i64(100).put_f64(2.72).finish());
        let rows: Vec<_> = t.rows(0).collect();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].col_i64(0), 42);
    }

    #[test]
    fn row_writer_with_str() {
        let schema = Schema::new()
            .col("ts", DType::I64)
            .col("msg", DType::Str)
            .col("tag", DType::U32);
        let mut t = MemTable::new(&schema, 4096, 1).unwrap();
        t.row_writer()
            .put_i64(1000)
            .put_str("hello")
            .put_u32(7)
            .finish();
        let row = t.rows(0).next().unwrap();
        assert_eq!(row.col_i64(0), 1000);
        assert_eq!(row.col_str(1), "hello");
        assert_eq!(row.col_u32(2), 7);
    }

    #[test]
    fn row_writer_overflow() {
        let schema = Schema::new().col("x", DType::I64);
        // ChunkHeader=40, each I64 row=12 → 56-40=16 → 1 row fits, 2nd overflows
        let mut t = MemTable::new(&schema, 56, 1).unwrap();
        assert!(t.row_writer().put_i64(1).finish());
        assert!(!t.row_writer().put_i64(2).finish());
        assert_eq!(t.num_rows(0), 1);
    }

    #[test]
    fn row_writer_drop_without_finish_commits_nothing() {
        let schema = Schema::new().col("x", DType::I32);
        let mut t = MemTable::new(&schema, 1024, 1).unwrap();
        {
            let mut w = t.row_writer();
            w.put_i32(99); // dropped without finish() → row not committed
        }
        assert_eq!(t.num_rows(0), 0, "uncommitted row must not be visible");
        // A subsequent write still works and is the first visible row.
        t.push_row(&[Value::I32(42)]);
        assert_eq!(t.num_rows(0), 1);
        assert_eq!(t.rows(0).next().unwrap().col_i32(0), 42);
    }

    #[test]
    fn row_writer_rejects_wrong_column_count_and_types() {
        let schema = Schema::new().col("x", DType::I32).col("label", DType::Str);
        let mut t = MemTable::new(&schema, 1024, 1).unwrap();

        assert!(!t.row_writer().put_i32(1).finish(), "missing column");
        assert!(
            !t.row_writer().put_i64(1).put_str("wrong").finish(),
            "wrong dtype"
        );
        assert!(
            !t.row_writer()
                .put_i32(1)
                .put_str("extra")
                .put_u8(1)
                .finish(),
            "extra column"
        );
        assert_eq!(t.num_rows(0), 0);
        assert!(t.row_writer().put_i32(7).put_str("ok").finish());
        assert_eq!(t.num_rows(0), 1);
        assert!(crate::validate_buf(t.as_bytes()).is_ok());
    }

    #[test]
    fn writer_and_value_interop() {
        let schema = Schema::new().col("x", DType::I64).col("s", DType::Str);
        let mut t = MemTable::new(&schema, 4096, 1).unwrap();
        t.row_writer().put_i64(1).put_str("writer").finish();
        t.push_row(&[Value::I64(2), Value::Str("value")]);
        let rows: Vec<_> = t.rows(0).collect();
        let mut c0 = rows[0].cursor();
        assert_eq!(c0.next_i64(), 1);
        assert_eq!(c0.next_str(), "writer");
        let mut c1 = rows[1].cursor();
        assert_eq!(c1.next_i64(), 2);
        assert_eq!(c1.next_str(), "value");
    }

    #[test]
    fn mixed_push_and_row_writer() {
        let schema = Schema::new().col("x", DType::I32);
        let mut t = MemTable::new(&schema, 1024, 1).unwrap();
        t.push_row(&[Value::I32(1)]);
        t.row_writer().put_i32(2).finish();
        assert_eq!(t.num_rows(0), 2);
        let rows: Vec<_> = t.rows(0).collect();
        assert_eq!(rows[0].col_i32(0), 1);
        assert_eq!(rows[1].col_i32(0), 2);
    }

    #[test]
    fn dedup_overflow_rolls_back_staged_entries() {
        let schema = Schema::new().col("value", DType::Str);
        let size = MemTable::required_size(&schema, 80, 1);
        let mut buf = vec![0u8; size];
        let mut writer = MemTableWriter::init(&mut buf, &schema, 80, 1)
            .unwrap()
            .dedup();

        assert!(!writer
            .row_writer()
            .put_str("same")
            .put_bytes(&[7; 128])
            .finish());
        assert!(writer.row_writer().put_str("same").finish());

        let row = writer.rows(0).next().expect("committed row");
        assert_eq!(row.col_str(0), "same");
        assert!(crate::validate_buf(writer.as_bytes()).is_ok());
    }

    #[test]
    fn dropped_dedup_row_rolls_back_staged_entries() {
        let schema = Schema::new().col("value", DType::Str);
        let size = MemTable::required_size(&schema, 128, 1);
        let mut buf = vec![0u8; size];
        let mut writer = MemTableWriter::init(&mut buf, &schema, 128, 1)
            .unwrap()
            .dedup();

        {
            let mut row = writer.row_writer();
            row.put_str("same");
        }
        assert!(writer.row_writer().put_str("same").finish());

        let row = writer.rows(0).next().expect("committed row");
        assert_eq!(row.col_str(0), "same");
        assert!(crate::validate_buf(writer.as_bytes()).is_ok());
    }
}
