//! 集成测试：仅使用 crate 公开 API。
//!
//! 长时间混沌压测见同目录 `chaos_stress.rs`，需加 `--ignored` 运行。

use probing_memtable::{DType, MemTable, Schema, Value};

#[test]
fn push_row_and_scan_rows() {
    let schema = Schema::new().col("id", DType::I64).col("msg", DType::Str);
    let mut t = MemTable::new(&schema, 4096, 2);
    t.push_row(&[Value::I64(1), Value::Str("hello")]);
    let row = t.rows(0).next().expect("one row");
    assert_eq!(row.col_i64(0), 1);
    assert_eq!(row.col_str(1), "hello");
}

#[test]
fn from_buf_roundtrip() {
    let schema = Schema::new().col("x", DType::I32);
    let mut t = MemTable::new(&schema, 1024, 1);
    t.push_row(&[Value::I32(7)]);
    let raw = t.as_bytes().to_vec();
    let t2 = MemTable::from_buf(raw).expect("valid buffer");
    assert_eq!(t2.num_rows(0), 1);
    assert_eq!(t2.rows(0).next().unwrap().col_i32(0), 7);
}
