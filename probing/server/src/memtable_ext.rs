use std::sync::Arc;

use datafusion::arrow::array::{
    ArrayRef, BinaryBuilder, Float32Builder, Float64Builder, GenericStringBuilder, Int32Builder,
    Int64Builder, RecordBatch, UInt32Builder, UInt64Builder, UInt8Builder,
};
use datafusion::arrow::datatypes::{DataType, Field, Schema, SchemaRef};

use probing_core::core::{
    CustomNamespace, EngineCall, EngineDatasource, EngineError, EngineExtension,
    EngineExtensionOption, LazyTableSource, NamespacePluginHelper, Plugin,
};
use probing_memtable::discover::default_dir;
use probing_memtable::{detect_table, DType, MemTableView, MemhView, TableKind, TypedValue};

fn self_dir() -> std::path::PathBuf {
    default_dir().join(std::process::id().to_string())
}

fn dtype_to_arrow(dt: DType) -> DataType {
    match dt {
        DType::U8 => DataType::UInt8,
        DType::U32 => DataType::UInt32,
        DType::I32 => DataType::Int32,
        DType::I64 => DataType::Int64,
        DType::F32 => DataType::Float32,
        DType::F64 => DataType::Float64,
        DType::U64 => DataType::UInt64,
        DType::Str => DataType::Utf8,
        DType::Bytes => DataType::Binary,
    }
}

fn view_to_arrow_schema(view: &MemTableView) -> SchemaRef {
    let s = view.schema();
    let fields: Vec<Field> = s
        .cols
        .iter()
        .map(|c| Field::new(&c.name, dtype_to_arrow(c.dtype), true))
        .collect();
    SchemaRef::new(Schema::new(fields))
}

enum ColBuilder {
    U8(UInt8Builder),
    U32(UInt32Builder),
    I32(Int32Builder),
    I64(Int64Builder),
    F32(Float32Builder),
    F64(Float64Builder),
    U64(UInt64Builder),
    Str(GenericStringBuilder<i32>),
    Bytes(BinaryBuilder),
}

fn view_to_recordbatch(view: &MemTableView) -> Vec<RecordBatch> {
    let schema = view.schema();
    let arrow_schema = view_to_arrow_schema(view);

    let mut builders: Vec<ColBuilder> = schema
        .cols
        .iter()
        .map(|c| match c.dtype {
            DType::U8 => ColBuilder::U8(UInt8Builder::new()),
            DType::U32 => ColBuilder::U32(UInt32Builder::new()),
            DType::I32 => ColBuilder::I32(Int32Builder::new()),
            DType::I64 => ColBuilder::I64(Int64Builder::new()),
            DType::F32 => ColBuilder::F32(Float32Builder::new()),
            DType::F64 => ColBuilder::F64(Float64Builder::new()),
            DType::U64 => ColBuilder::U64(UInt64Builder::new()),
            DType::Str => ColBuilder::Str(GenericStringBuilder::new()),
            DType::Bytes => ColBuilder::Bytes(BinaryBuilder::new()),
        })
        .collect();

    for chunk in 0..view.num_chunks() {
        for row in view.rows(chunk) {
            let mut cursor = row.cursor();
            for builder in builders.iter_mut() {
                match builder {
                    ColBuilder::U8(b) => b.append_value(cursor.next_u8()),
                    ColBuilder::U32(b) => b.append_value(cursor.next_u32()),
                    ColBuilder::I32(b) => b.append_value(cursor.next_i32()),
                    ColBuilder::I64(b) => b.append_value(cursor.next_i64()),
                    ColBuilder::F32(b) => b.append_value(cursor.next_f32()),
                    ColBuilder::F64(b) => b.append_value(cursor.next_f64()),
                    ColBuilder::U64(b) => b.append_value(cursor.next_u64()),
                    ColBuilder::Str(b) => b.append_value(cursor.next_str()),
                    ColBuilder::Bytes(b) => b.append_value(cursor.next_bytes()),
                }
            }
        }
    }

    let arrays: Vec<ArrayRef> = builders
        .into_iter()
        .map(|b| -> ArrayRef {
            match b {
                ColBuilder::U8(mut b) => Arc::new(b.finish()),
                ColBuilder::U32(mut b) => Arc::new(b.finish()),
                ColBuilder::I32(mut b) => Arc::new(b.finish()),
                ColBuilder::I64(mut b) => Arc::new(b.finish()),
                ColBuilder::F32(mut b) => Arc::new(b.finish()),
                ColBuilder::F64(mut b) => Arc::new(b.finish()),
                ColBuilder::U64(mut b) => Arc::new(b.finish()),
                ColBuilder::Str(mut b) => Arc::new(b.finish()),
                ColBuilder::Bytes(mut b) => Arc::new(b.finish()),
            }
        })
        .collect();

    match RecordBatch::try_new(arrow_schema, arrays) {
        Ok(batch) => vec![batch],
        Err(e) => {
            log::error!("memtable → RecordBatch failed: {e}");
            vec![]
        }
    }
}

// ── MEMH: key-value table → two-column RecordBatch ────────────────────

/// Fixed Arrow schema for MEMH tables: `key` (Utf8) + `value` (Utf8).
///
/// All MEMH values are serialised to strings so that heterogeneous value types
/// (scalars, strings, bytes) can be represented in a single column and queried
/// with SQL string predicates.
fn memh_kv_schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("key", DataType::Utf8, false),
        Field::new("value", DataType::Utf8, true),
    ]))
}

fn typed_value_to_str(v: &TypedValue<'_>) -> String {
    match v {
        TypedValue::U8(n)  => n.to_string(),
        TypedValue::I32(n) => n.to_string(),
        TypedValue::I64(n) => n.to_string(),
        TypedValue::F32(n) => n.to_string(),
        TypedValue::F64(n) => n.to_string(),
        TypedValue::U64(n) => n.to_string(),
        TypedValue::U32(n) => n.to_string(),
        TypedValue::Str(s) => s.to_string(),
        TypedValue::Bytes(b) => {
            // Hex-encode without adding a dep; e.g. "0xdeadbeef"
            let mut out = String::with_capacity(2 + b.len() * 2);
            out.push_str("0x");
            for byte in *b {
                use std::fmt::Write;
                let _ = write!(out, "{byte:02x}");
            }
            out
        }
    }
}

fn memh_view_to_recordbatch(view: &MemhView<'_>) -> Vec<RecordBatch> {
    let schema = memh_kv_schema();
    let mut keys:   GenericStringBuilder<i32> = GenericStringBuilder::new();
    let mut values: GenericStringBuilder<i32> = GenericStringBuilder::new();

    for (k, v) in view.iter() {
        keys.append_value(k);
        values.append_value(typed_value_to_str(&v));
    }

    match RecordBatch::try_new(
        schema,
        vec![Arc::new(keys.finish()), Arc::new(values.finish())],
    ) {
        Ok(batch) => vec![batch],
        Err(e) => {
            log::error!("memh → RecordBatch failed: {e}");
            vec![]
        }
    }
}

// ── CustomNamespace ────────────────────────────────────────────────────

#[derive(Default, Debug)]
pub struct MemTableNamespace;

impl CustomNamespace for MemTableNamespace {
    fn name() -> &'static str {
        "memtable"
    }

    fn list() -> Vec<String> {
        let dir = self_dir();
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => return vec![],
        };
        entries
            .flatten()
            .filter(|e| e.path().is_file())
            .filter_map(|e| Some(e.file_name().to_string_lossy().to_string()))
            .collect()
    }

    fn make_lazy(expr: &str) -> Arc<LazyTableSource> {
        let path = self_dir().join(expr);
        let data = match std::fs::read(&path) {
            Ok(d) => d,
            Err(_) => return Arc::new(LazyTableSource::default()),
        };

        match detect_table(&data) {
            Some(TableKind::Ring) => {
                let view = match MemTableView::new(&data) {
                    Ok(v) => v,
                    Err(_) => return Arc::new(LazyTableSource::default()),
                };
                Arc::new(LazyTableSource {
                    name: expr.to_string(),
                    schema: Some(view_to_arrow_schema(&view)),
                    data: view_to_recordbatch(&view),
                })
            }
            Some(TableKind::Hash) => {
                let view = match MemhView::new(&data) {
                    Ok(v) => v,
                    Err(_) => return Arc::new(LazyTableSource::default()),
                };
                Arc::new(LazyTableSource {
                    name: expr.to_string(),
                    schema: Some(memh_kv_schema()),
                    data: memh_view_to_recordbatch(&view),
                })
            }
            None => Arc::new(LazyTableSource::default()),
        }
    }
}

// ── EngineExtension ────────────────────────────────────────────────────

pub type MemTablePlugin = NamespacePluginHelper<MemTableNamespace>;

#[derive(Debug, Default, EngineExtension)]
pub struct MemTableExtension {}

impl EngineCall for MemTableExtension {}

impl EngineDatasource for MemTableExtension {
    fn datasrc(
        &self,
        namespace: &str,
        _name: Option<&str>,
    ) -> Option<Arc<dyn Plugin + Sync + Send>> {
        Some(MemTablePlugin::create(namespace))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use datafusion::arrow::array::{
        AsArray, Float64Array, Int32Array, Int64Array, UInt8Array,
    };
    use probing_memtable::{MemTable, Schema as MtSchema, Value};

    #[test]
    fn dtype_mapping_covers_all_variants() {
        assert_eq!(dtype_to_arrow(DType::U8), DataType::UInt8);
        assert_eq!(dtype_to_arrow(DType::U32), DataType::UInt32);
        assert_eq!(dtype_to_arrow(DType::I32), DataType::Int32);
        assert_eq!(dtype_to_arrow(DType::I64), DataType::Int64);
        assert_eq!(dtype_to_arrow(DType::F32), DataType::Float32);
        assert_eq!(dtype_to_arrow(DType::F64), DataType::Float64);
        assert_eq!(dtype_to_arrow(DType::U64), DataType::UInt64);
        assert_eq!(dtype_to_arrow(DType::Str), DataType::Utf8);
        assert_eq!(dtype_to_arrow(DType::Bytes), DataType::Binary);
    }

    #[test]
    fn recordbatch_from_mixed_types() {
        let schema = MtSchema::new()
            .col("id", DType::I32)
            .col("value", DType::F64)
            .col("tag", DType::Str);
        let mut t = MemTable::new(&schema, 4096, 2);
        t.push_row(&[Value::I32(1), Value::F64(3.14), Value::Str("hello")]);
        t.push_row(&[Value::I32(2), Value::F64(2.72), Value::Str("world")]);

        let view = t.view();
        let batches = view_to_recordbatch(&view);
        assert_eq!(batches.len(), 1);
        let batch = &batches[0];
        assert_eq!(batch.num_rows(), 2);
        assert_eq!(batch.num_columns(), 3);

        let ids = batch.column(0).as_any().downcast_ref::<Int32Array>().unwrap();
        assert_eq!(ids.value(0), 1);
        assert_eq!(ids.value(1), 2);

        let vals = batch.column(1).as_any().downcast_ref::<Float64Array>().unwrap();
        assert!((vals.value(0) - 3.14).abs() < 1e-10);
        assert!((vals.value(1) - 2.72).abs() < 1e-10);

        let tags: &datafusion::arrow::array::StringArray = batch.column(2).as_string();
        assert_eq!(tags.value(0), "hello");
        assert_eq!(tags.value(1), "world");
    }

    #[test]
    fn recordbatch_multiple_chunks() {
        let schema = MtSchema::new().col("v", DType::I64);
        // Small chunk so rows spill across chunks
        let mut t = MemTable::new(&schema, 128, 4);
        for i in 0..20 {
            t.push_row(&[Value::I64(i)]);
        }

        let view = t.view();
        let batches = view_to_recordbatch(&view);
        assert_eq!(batches.len(), 1);
        let batch = &batches[0];
        // Ring buffer may have overwritten old chunks, but total rows should be > 0
        assert!(batch.num_rows() > 0);

        let col = batch.column(0).as_any().downcast_ref::<Int64Array>().unwrap();
        // Verify values are sequential (from whatever chunks survived)
        for i in 1..col.len() {
            assert!(col.value(i) > col.value(i - 1));
        }
    }

    #[test]
    fn recordbatch_empty_table() {
        let schema = MtSchema::new().col("x", DType::U8);
        let t = MemTable::new(&schema, 1024, 1);
        let view = t.view();
        let batches = view_to_recordbatch(&view);
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].num_rows(), 0);
    }

    #[test]
    fn arrow_schema_matches_memtable_schema() {
        let schema = MtSchema::new()
            .col("ts", DType::I64)
            .col("cpu", DType::F64)
            .col("name", DType::Str);
        let t = MemTable::new(&schema, 1024, 1);
        let view = t.view();
        let arrow = view_to_arrow_schema(&view);

        assert_eq!(arrow.fields().len(), 3);
        assert_eq!(arrow.field(0).name(), "ts");
        assert_eq!(*arrow.field(0).data_type(), DataType::Int64);
        assert_eq!(arrow.field(1).name(), "cpu");
        assert_eq!(*arrow.field(1).data_type(), DataType::Float64);
        assert_eq!(arrow.field(2).name(), "name");
        assert_eq!(*arrow.field(2).data_type(), DataType::Utf8);
    }

    #[test]
    fn recordbatch_u8_column() {
        let schema = MtSchema::new().col("flag", DType::U8);
        let mut t = MemTable::new(&schema, 1024, 1);
        t.push_row(&[Value::U8(0)]);
        t.push_row(&[Value::U8(255)]);

        let view = t.view();
        let batches = view_to_recordbatch(&view);
        let col = batches[0].column(0).as_any().downcast_ref::<UInt8Array>().unwrap();
        assert_eq!(col.value(0), 0);
        assert_eq!(col.value(1), 255);
    }

    #[test]
    fn namespace_list_and_make_lazy_via_exposed_table() {
        use probing_memtable::discover::ExposedTable;

        let tmp = tempfile::tempdir().unwrap();
        // Override discovery dir via env var
        let orig = std::env::var("PROBING_DATA_DIR").ok();
        std::env::set_var("PROBING_DATA_DIR", tmp.path());

        let schema = MtSchema::new()
            .col("ts", DType::I64)
            .col("msg", DType::Str);
        let mut table = ExposedTable::create("test_metrics", &schema, 4096, 2).unwrap();
        {
            let mut w = table.writer();
            w.push_row(&[Value::I64(100), Value::Str("alpha")]);
            w.push_row(&[Value::I64(200), Value::Str("beta")]);
        }

        // list() should find the table
        let names = MemTableNamespace::list();
        assert!(names.contains(&"test_metrics".to_string()), "got: {names:?}");

        // make_lazy() should read data correctly
        let lazy = MemTableNamespace::make_lazy("test_metrics");
        assert_eq!(lazy.data.len(), 1);
        let batch = &lazy.data[0];
        assert_eq!(batch.num_rows(), 2);

        let ts = batch.column(0).as_any().downcast_ref::<Int64Array>().unwrap();
        assert_eq!(ts.value(0), 100);
        assert_eq!(ts.value(1), 200);

        let msgs: &datafusion::arrow::array::StringArray = batch.column(1).as_string();
        assert_eq!(msgs.value(0), "alpha");
        assert_eq!(msgs.value(1), "beta");

        // Cleanup
        drop(table);
        match orig {
            Some(v) => std::env::set_var("PROBING_DATA_DIR", v),
            None => std::env::remove_var("PROBING_DATA_DIR"),
        }
    }
}
