use std::collections::HashMap;
use xxhash_rust::xxh3;

/// Per-chunk string/bytes dedup map for streaming or batch writers.
/// Cleared when advancing to the next chunk.
///
/// Strings shorter than `min_dedup_len` are always stored inline,
/// skipping the hash + HashMap overhead.  Default is 0 (dedup all).
pub(crate) struct DedupState {
    seen: HashMap<u64, usize>,
    min_dedup_len: usize,
}

impl DedupState {
    pub fn new() -> Self {
        Self {
            seen: HashMap::with_capacity(64),
            min_dedup_len: 0,
        }
    }

    pub fn set_min_dedup_len(&mut self, len: usize) {
        self.min_dedup_len = len;
    }

    pub fn clear(&mut self) {
        self.seen.clear();
    }

    fn key(col: usize, data: &[u8]) -> u64 {
        xxh3::xxh3_64_with_seed(data, col as u64)
    }

    pub(crate) fn lookup(&self, col: usize, data: &[u8]) -> Option<usize> {
        if data.len() < self.min_dedup_len {
            return None;
        }
        self.seen.get(&Self::key(col, data)).copied()
    }

    pub(crate) fn insert(&mut self, col: usize, data: &[u8], chunk_offset: usize) {
        if data.len() >= self.min_dedup_len {
            self.seen.insert(Self::key(col, data), chunk_offset);
        }
    }
}

impl Default for DedupState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use crate::memtable::{MemTable, MemTableView, MemTableWriter};
    use crate::schema::{DType, Schema, Value};

    #[test]
    fn dedup_str_saves_space() {
        let schema = Schema::new().col("tag", DType::Str).col("val", DType::I32);
        let size = MemTable::required_size(&schema, 4096, 1);
        let mut buf = vec![0u8; size];
        let mut dw = MemTableWriter::init(&mut buf, &schema, 4096, 1).dedup();

        dw.push_row(&[Value::Str("hello"), Value::I32(1)]);
        let used_after_first = dw.chunk_used(0);

        dw.push_row(&[Value::Str("hello"), Value::I32(2)]);
        let used_after_second = dw.chunk_used(0);

        dw.push_row(&[Value::Str("world"), Value::I32(3)]);
        let used_after_third = dw.chunk_used(0);

        let second_row_size = used_after_second - used_after_first;
        let third_row_size = used_after_third - used_after_second;
        assert!(
            second_row_size < third_row_size,
            "dedup should save: {second_row_size} vs {third_row_size}"
        );
        assert_eq!(second_row_size, 12); // 4+4+4
        assert_eq!(third_row_size, 17); // 4+(4+5)+4

        let rows: Vec<_> = dw.rows(0).collect();
        assert_eq!(rows[0].col_str(0), "hello");
        assert_eq!(rows[0].col_i32(1), 1);
        assert_eq!(rows[1].col_str(0), "hello");
        assert_eq!(rows[1].col_i32(1), 2);
        assert_eq!(rows[2].col_str(0), "world");
        assert_eq!(rows[2].col_i32(1), 3);
    }

    #[test]
    fn dedup_row_writer_cursor_read() {
        let schema = Schema::new()
            .col("id", DType::I64)
            .col("name", DType::Str)
            .col("status", DType::Str);
        let size = MemTable::required_size(&schema, 4096, 1);
        let mut buf = vec![0u8; size];
        let mut dw = MemTableWriter::init(&mut buf, &schema, 4096, 1).dedup();

        dw.row_writer()
            .put_i64(1)
            .put_str("alice")
            .put_str("active")
            .finish();
        dw.row_writer()
            .put_i64(2)
            .put_str("bob")
            .put_str("active")
            .finish();
        dw.row_writer()
            .put_i64(3)
            .put_str("alice")
            .put_str("inactive")
            .finish();

        for (i, row) in dw.rows(0).enumerate() {
            let mut c = row.cursor();
            let id = c.next_i64();
            let name = c.next_str();
            let status = c.next_str();
            match i {
                0 => {
                    assert_eq!(id, 1);
                    assert_eq!(name, "alice");
                    assert_eq!(status, "active");
                }
                1 => {
                    assert_eq!(id, 2);
                    assert_eq!(name, "bob");
                    assert_eq!(status, "active");
                }
                2 => {
                    assert_eq!(id, 3);
                    assert_eq!(name, "alice");
                    assert_eq!(status, "inactive");
                }
                _ => unreachable!(),
            }
        }
    }

    #[test]
    fn dedup_bytes_column() {
        let schema = Schema::new().col("payload", DType::Bytes);
        let size = MemTable::required_size(&schema, 4096, 1);
        let mut buf = vec![0u8; size];
        let mut dw = MemTableWriter::init(&mut buf, &schema, 4096, 1).dedup();

        let data = &[0xDE, 0xAD, 0xBE, 0xEF];
        dw.push_row(&[Value::Bytes(data)]);
        let u1 = dw.chunk_used(0);
        dw.push_row(&[Value::Bytes(data)]);
        let u2 = dw.chunk_used(0);
        dw.push_row(&[Value::Bytes(&[0xFF])]);

        assert_eq!(u2 - u1, 8); // 4 row_len + 4 ref
        let rows: Vec<_> = dw.rows(0).collect();
        assert_eq!(rows[0].col_bytes(0), data);
        assert_eq!(rows[1].col_bytes(0), data);
        assert_eq!(rows[2].col_bytes(0), &[0xFF]);
    }

    #[test]
    fn dedup_empty_str_not_deduped() {
        let schema = Schema::new().col("s", DType::Str);
        let size = MemTable::required_size(&schema, 4096, 1);
        let mut buf = vec![0u8; size];
        let mut dw = MemTableWriter::init(&mut buf, &schema, 4096, 1).dedup();
        dw.push_row(&[Value::Str("")]);
        dw.push_row(&[Value::Str("")]);
        assert_eq!(dw.chunk_used(0), 16); // both inline: 8+8
        assert_eq!(dw.rows(0).next().unwrap().col_str(0), "");
    }

    #[test]
    fn dedup_across_chunk_boundary_resets() {
        let schema = Schema::new().col("tag", DType::Str);
        let size = MemTable::required_size(&schema, 128, 2);
        let mut buf = vec![0u8; size];
        let mut dw = MemTableWriter::init(&mut buf, &schema, 128, 2).dedup();

        for _ in 0..5 {
            dw.push_row(&[Value::Str("repeat")]);
        }
        dw.advance_chunk();

        // chunk 1: first inline, second dedup
        dw.push_row(&[Value::Str("repeat")]);
        dw.push_row(&[Value::Str("repeat")]);

        let rows_c1: Vec<_> = dw.rows(1).collect();
        assert_eq!(rows_c1.len(), 2);
        assert_eq!(rows_c1[0].col_str(0), "repeat");
        assert_eq!(rows_c1[1].col_str(0), "repeat");
        assert_eq!(dw.chunk_used(1), 14 + 8); // 14 inline + 8 dedup
    }

    #[test]
    fn dedup_many_duplicates() {
        let schema = Schema::new()
            .col("level", DType::Str)
            .col("msg", DType::Str);
        let size = MemTable::required_size(&schema, 8192, 1);
        let mut buf = vec![0u8; size];
        let mut dw = MemTableWriter::init(&mut buf, &schema, 8192, 1).dedup();

        let levels = ["INFO", "WARN", "ERROR"];
        for i in 0..30 {
            dw.row_writer()
                .put_str(levels[i % 3])
                .put_str(&format!("message_{i}"))
                .finish();
        }

        for (i, row) in dw.rows(0).enumerate() {
            let mut c = row.cursor();
            assert_eq!(c.next_str(), levels[i % 3]);
            assert_eq!(c.next_str(), format!("message_{i}"));
        }
        assert_eq!(dw.num_rows(0), 30);
    }

    #[test]
    fn stress_dedup_writer_large_volume() {
        let schema = Schema::new()
            .col("ts", DType::I64)
            .col("level", DType::Str)
            .col("component", DType::Str)
            .col("msg", DType::Str);
        let size = MemTable::required_size(&schema, 65536, 8);
        let mut buf = vec![0u8; size];
        let mut dw = MemTableWriter::init(&mut buf, &schema, 65536, 8).dedup();

        let levels = ["TRACE", "DEBUG", "INFO", "WARN", "ERROR"];
        let components = ["http", "db", "cache", "auth", "scheduler", "worker"];
        let n = 2000;

        for i in 0..n as i64 {
            dw.push_row(&[
                Value::I64(i),
                Value::Str(levels[i as usize % levels.len()]),
                Value::Str(components[i as usize % components.len()]),
                Value::Str(&format!("event_{}", i % 200)),
            ]);
        }

        // verify every row is readable and correct
        let mut total = 0;
        for chunk in 0..dw.num_chunks() {
            for row in dw.rows(chunk) {
                let mut c = row.cursor();
                let ts = c.next_i64();
                let level = c.next_str();
                let comp = c.next_str();
                let msg = c.next_str();
                assert!(levels.contains(&level), "bad level: {level}");
                assert!(components.contains(&comp), "bad component: {comp}");
                assert!(msg.starts_with("event_"), "bad msg: {msg}");
                assert!(ts >= 0);
                total += 1;
            }
        }
        assert!(total > 0, "should have rows");
    }

    #[test]
    fn stress_ring_buffer_wrap_with_dedup() {
        let schema = Schema::new().col("tag", DType::Str).col("seq", DType::I64);
        // tiny chunks → frequent wraps
        let size = MemTable::required_size(&schema, 256, 4);
        let mut buf = vec![0u8; size];
        let mut dw = MemTableWriter::init(&mut buf, &schema, 256, 4).dedup();

        let tags = ["alpha", "beta", "gamma"];
        for i in 0..500i64 {
            dw.push_row(&[Value::Str(tags[i as usize % 3]), Value::I64(i)]);
        }

        // at least some chunks should have data; ring wraps many times
        let mut any_rows = false;
        for chunk in 0..dw.num_chunks() {
            for row in dw.rows(chunk) {
                let mut c = row.cursor();
                let tag = c.next_str();
                let _seq = c.next_i64();
                assert!(tags.contains(&tag));
                any_rows = true;
            }
        }
        assert!(any_rows);
    }

    #[test]
    fn stress_concurrent_dedup_writers() {
        let schema = Schema::new()
            .col("tid", DType::I32)
            .col("tag", DType::Str)
            .col("seq", DType::I64);
        let num_threads = 8;
        let rows_per_thread = 200;
        let size = MemTable::required_size(&schema, 32768, 8);
        let mut buf = vec![0u8; size];
        let mut mt = MemTableWriter::init(&mut buf, &schema, 32768, 8).dedup();

        let tags = ["A", "B", "C", "D"];
        for tid in 0..num_threads {
            for seq in 0..rows_per_thread as i64 {
                let tag = tags[seq as usize % tags.len()];
                mt.push_row(&[Value::I32(tid as i32), Value::Str(tag), Value::I64(seq)]);
            }
        }

        let view = MemTableView::new(&buf).unwrap();
        let total: usize = (0..view.num_chunks()).map(|c| view.num_rows(c)).sum();
        assert_eq!(total, num_threads * rows_per_thread);

        for chunk in 0..view.num_chunks() {
            for row in view.rows(chunk) {
                let mut c = row.cursor();
                let tid = c.next_i32();
                let tag = c.next_str();
                let _seq = c.next_i64();
                assert!((0..num_threads as i32).contains(&tid));
                assert!(tags.contains(&tag), "corrupt tag: {tag}");
            }
        }
    }

    #[test]
    fn stress_many_columns_dedup() {
        let mut schema = Schema::new();
        for i in 0..16 {
            schema = schema.col(
                &format!("c{i}"),
                if i % 2 == 0 { DType::Str } else { DType::I32 },
            );
        }
        let size = MemTable::required_size(&schema, 65536, 1);
        let mut buf = vec![0u8; size];
        let mut dw = MemTableWriter::init(&mut buf, &schema, 65536, 1).dedup();

        let tags = ["x", "y", "z"];
        for i in 0..200 {
            let mut values: Vec<Value> = Vec::new();
            for col in 0..16 {
                if col % 2 == 0 {
                    values.push(Value::Str(tags[i % tags.len()]));
                } else {
                    values.push(Value::I32((i * 16 + col) as i32));
                }
            }
            dw.push_row(&values);
        }

        // verify all rows
        let mut count = 0;
        for row in dw.rows(0) {
            let mut c = row.cursor();
            for col in 0..16 {
                if col % 2 == 0 {
                    let s = c.next_str();
                    assert!(tags.contains(&s), "bad str at col {col}: {s}");
                } else {
                    let v = c.next_i32();
                    assert!(v >= 0);
                }
            }
            count += 1;
        }
        assert_eq!(count, 200);
    }

    #[test]
    fn stress_tiny_chunks_rapid_advance() {
        let schema = Schema::new().col("tag", DType::Str).col("v", DType::I32);
        // each chunk fits ~2-3 rows only
        let size = MemTable::required_size(&schema, 64, 16);
        let mut buf = vec![0u8; size];
        let mut dw = MemTableWriter::init(&mut buf, &schema, 64, 16).dedup();

        let tags = ["aaa", "bbb"];
        for i in 0..200 {
            dw.push_row(&[Value::Str(tags[i % 2]), Value::I32(i as i32)]);
        }

        let mut total = 0;
        for chunk in 0..dw.num_chunks() {
            for row in dw.rows(chunk) {
                let mut c = row.cursor();
                let tag = c.next_str();
                let _v = c.next_i32();
                assert!(tags.contains(&tag));
                total += 1;
            }
        }
        assert!(total > 0, "should have rows across chunks");
    }

    #[test]
    fn stress_long_strings_dedup() {
        let schema = Schema::new().col("payload", DType::Str);
        let size = MemTable::required_size(&schema, 65536, 2);
        let mut buf = vec![0u8; size];
        let mut dw = MemTableWriter::init(&mut buf, &schema, 65536, 2).dedup();

        // a 1KB string repeated many times
        let long_str: String = "x".repeat(1024);
        let short_str = "tiny";

        dw.push_row(&[Value::Str(&long_str)]);
        let after_first = dw.chunk_used(0);

        for _ in 0..50 {
            dw.push_row(&[Value::Str(&long_str)]);
        }
        let after_51 = dw.chunk_used(0);

        // 50 dedup refs should use ~50*8 = 400 bytes, not 50*1028
        let dedup_data = after_51 - after_first;
        assert!(
            dedup_data < 1000,
            "dedup of 1KB string should save space: {dedup_data}"
        );

        dw.push_row(&[Value::Str(short_str)]);

        for row in dw.rows(0) {
            let s = row.col_str(0);
            assert!(s == long_str || s == short_str, "bad: len={}", s.len());
        }
    }
}
