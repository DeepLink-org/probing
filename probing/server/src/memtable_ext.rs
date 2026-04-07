//! Mmap memtable integration for DataFusion.
//!
//! ## File → SQL mapping (no hard-coded product prefix)
//!
//! Each regular file under `<data_dir>/<pid>/` can be queried when its name is valid:
//!
//! - **First `.` splits schema vs table** — `acme.actors` → schema `acme`, table `actors`;
//!   `foo.bar.baz` → schema `foo`, table `bar.baz` (on-disk name is the full filename).
//! - **No `.`** — exposed as `memtable.<filename>` (e.g. `metrics` → `memtable.metrics`).
//!
//! Schema head and table tail must be non-empty; only ASCII letters, digits, `_`, and
//! `.` inside the table tail are allowed (no `/`, `\\`). Leading-dot names are ignored.
use std::any::Any;
use std::collections::BTreeSet;
use std::sync::Arc;

use async_trait::async_trait;
use datafusion::arrow::array::{
    ArrayRef, BinaryBuilder, Float32Builder, Float64Builder, GenericStringBuilder, Int32Builder,
    Int64Builder, RecordBatch, UInt32Builder, UInt64Builder, UInt8Builder,
};
use datafusion::arrow::datatypes::{DataType, Field, Schema, SchemaRef};
use datafusion::catalog::CatalogProvider;
use datafusion::catalog::SchemaProvider;
use datafusion::datasource::TableProvider;
use datafusion::error::DataFusionError;
use datafusion::error::Result as DfResult;

use probing_core::core::{
    EngineCall, EngineDatasource, EngineError, EngineExtension, EngineExtensionOption,
    LazyTableSource, Plugin, PluginType,
};
use probing_memtable::discover::default_dir;
use probing_memtable::{detect_table, DType, MemTableView, MemhView, TableKind, TypedValue};

/// SQL schema used for mmap files whose basename contains no `.`.
pub const DEFAULT_UNDOTTED_SCHEMA: &str = "memtable";

fn self_dir() -> std::path::PathBuf {
    default_dir().join(std::process::id().to_string())
}

#[inline]
fn valid_schema_head(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

#[inline]
fn valid_table_tail(s: &str) -> bool {
    !s.is_empty()
        && !s.contains('/')
        && !s.contains('\\')
        && s
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.')
}

/// Map basename `filename` → `(schema, table)` for routing; [`None`] if skipped.
pub fn classify_mmap_basename(filename: &str) -> Option<(String, String)> {
    if filename.starts_with('.') {
        return None;
    }
    if let Some((head, tail)) = filename.split_once('.') {
        if valid_schema_head(head) && valid_table_tail(tail) {
            return Some((head.to_string(), tail.to_string()));
        }
        return None;
    }
    if valid_schema_head(filename) {
        Some((DEFAULT_UNDOTTED_SCHEMA.to_string(), filename.to_string()))
    } else {
        None
    }
}

/// On-disk filename for a `(schema, table)` pair.
pub fn mmap_filename_for(schema: &str, table: &str) -> String {
    if schema == DEFAULT_UNDOTTED_SCHEMA {
        table.to_string()
    } else {
        format!("{schema}.{table}")
    }
}

fn tables_in_schema(target_schema: &str) -> Vec<String> {
    let dir = self_dir();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return vec![];
    };
    let mut out = Vec::new();
    for e in entries.flatten() {
        if !e.path().is_file() {
            continue;
        }
        let n = e.file_name().to_string_lossy().to_string();
        if let Some((sch, tbl)) = classify_mmap_basename(&n) {
            if sch == target_schema {
                out.push(tbl);
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

fn discover_all_schemas() -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let dir = self_dir();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            if !e.path().is_file() {
                continue;
            }
            let n = e.file_name().to_string_lossy().to_string();
            if let Some((sch, _)) = classify_mmap_basename(&n) {
                out.insert(sch);
            }
        }
    }
    out.insert(DEFAULT_UNDOTTED_SCHEMA.to_string());
    out
}

fn bytes_to_lazy_table(data: &[u8], logical_name: &str) -> Arc<LazyTableSource> {
    match detect_table(data) {
        Some(TableKind::Ring) => {
            let view = match MemTableView::new(data) {
                Ok(v) => v,
                Err(_) => return Arc::new(LazyTableSource::default()),
            };
            Arc::new(LazyTableSource {
                name: logical_name.to_string(),
                schema: Some(view_to_arrow_schema(&view)),
                data: view_to_recordbatch(&view),
            })
        }
        Some(TableKind::Hash) => {
            let view = match MemhView::new(data) {
                Ok(v) => v,
                Err(_) => return Arc::new(LazyTableSource::default()),
            };
            Arc::new(LazyTableSource {
                name: logical_name.to_string(),
                schema: Some(memh_kv_schema()),
                data: memh_view_to_recordbatch(&view),
            })
        }
        None => Arc::new(LazyTableSource::default()),
    }
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

// ── Dynamic schemas from mmap filenames ───────────────────────────────

/// One DataFusion schema: tables are mmap files whose basename maps here via
/// [`classify_mmap_basename`].
#[derive(Debug)]
pub struct MmapFileSchemaProvider {
    schema: String,
}

impl MmapFileSchemaProvider {
    pub fn new(schema: impl Into<String>) -> Self {
        Self {
            schema: schema.into(),
        }
    }
}

#[async_trait]
impl SchemaProvider for MmapFileSchemaProvider {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn table_names(&self) -> Vec<String> {
        tables_in_schema(&self.schema)
    }

    async fn table(&self, name: &str) -> DfResult<Option<Arc<dyn TableProvider>>> {
        let names = self.table_names();
        if !names.iter().any(|n| n == name) {
            return Ok(None);
        }
        let path = self_dir().join(mmap_filename_for(&self.schema, name));
        let data = match std::fs::read(&path) {
            Ok(d) => d,
            Err(_) => return Ok(None),
        };
        Ok(Some(bytes_to_lazy_table(&data, name)))
    }

    fn register_table(
        &self,
        _name: String,
        _table: Arc<dyn TableProvider>,
    ) -> DfResult<Option<Arc<dyn TableProvider>>> {
        Err(DataFusionError::NotImplemented(
            "unable to create tables".to_string(),
        ))
    }

    fn deregister_table(&self, _name: &str) -> DfResult<Option<Arc<dyn TableProvider>>> {
        Err(DataFusionError::NotImplemented(
            "unable to drop tables".to_string(),
        ))
    }

    fn table_exist(&self, name: &str) -> bool {
        self.table_names().iter().any(|n| n == name)
    }
}

/// Wraps `probe` catalog; delegates static schemas (python, cluster, …)
/// to inner, discovers mmap-backed schemas (e.g. `pulsing.*`) at query time.
#[derive(Debug)]
struct DynamicMmapCatalog {
    inner: Arc<dyn CatalogProvider>,
}

impl CatalogProvider for DynamicMmapCatalog {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn schema_names(&self) -> Vec<String> {
        let mut names: BTreeSet<String> = self.inner.schema_names().into_iter().collect();
        for sch in discover_all_schemas() {
            names.insert(sch);
        }
        names.into_iter().collect()
    }

    fn schema(&self, name: &str) -> Option<Arc<dyn SchemaProvider>> {
        if !tables_in_schema(name).is_empty() || name == DEFAULT_UNDOTTED_SCHEMA {
            return Some(Arc::new(MmapFileSchemaProvider::new(name)));
        }
        self.inner.schema(name)
    }

    fn register_schema(
        &self,
        name: &str,
        schema: Arc<dyn SchemaProvider>,
    ) -> DfResult<Option<Arc<dyn SchemaProvider>>> {
        self.inner.register_schema(name, schema)
    }
}

/// Namespace plugin that wraps the `probe` catalog with [`DynamicMmapCatalog`]
/// for dynamic schema discovery from mmap files at query time.
#[derive(Debug, Default)]
pub struct UnifiedMemtablePlugin;

impl Plugin for UnifiedMemtablePlugin {
    fn name(&self) -> String { "mmap_memtables".into() }
    fn kind(&self) -> PluginType { PluginType::Namespace }
    fn namespace(&self) -> String { "memtable".into() }

    fn provide_catalog(
        &self,
        inner: Arc<dyn CatalogProvider>,
    ) -> Option<Arc<dyn CatalogProvider>> {
        Some(Arc::new(DynamicMmapCatalog { inner }))
    }
}

// ── EngineExtension ────────────────────────────────────────────────────

#[derive(Debug, Default, EngineExtension)]
pub struct MemTableExtension {}

impl EngineCall for MemTableExtension {}

impl EngineDatasource for MemTableExtension {
    fn datasrc(
        &self,
        _namespace: &str,
        _name: Option<&str>,
    ) -> Option<Arc<dyn Plugin + Sync + Send>> {
        Some(Arc::new(UnifiedMemtablePlugin::default()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use datafusion::arrow::array::{
        AsArray, Float64Array, Int32Array, Int64Array, UInt8Array,
    };
    use probing_memtable::{MemTable, Schema as MtSchema, Value};
    use std::sync::Mutex;

    /// `PROBING_DATA_DIR` is process-global; serialize tests that mutate it.
    static PROBING_DATA_DIR_LOCK: Mutex<()> = Mutex::new(());

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

    fn read_lazy_from_mmap(schema: &str, table: &str) -> Arc<LazyTableSource> {
        let path = self_dir().join(mmap_filename_for(schema, table));
        let data = std::fs::read(path).unwrap();
        bytes_to_lazy_table(&data, table)
    }

    #[test]
    fn classify_and_mmap_roundtrip() {
        assert_eq!(
            classify_mmap_basename("pulsing.actors"),
            Some(("pulsing".into(), "actors".into()))
        );
        assert_eq!(
            classify_mmap_basename("foo.bar.baz"),
            Some(("foo".into(), "bar.baz".into()))
        );
        assert_eq!(
            classify_mmap_basename("metrics"),
            Some((DEFAULT_UNDOTTED_SCHEMA.into(), "metrics".into()))
        );
        assert_eq!(mmap_filename_for(DEFAULT_UNDOTTED_SCHEMA, "metrics"), "metrics");
        assert_eq!(mmap_filename_for("pulsing", "actors"), "pulsing.actors");
        assert_eq!(mmap_filename_for("foo", "bar.baz"), "foo.bar.baz");
    }

    #[test]
    fn namespace_list_and_make_lazy_via_exposed_table() {
        let _lock = PROBING_DATA_DIR_LOCK.lock().unwrap();
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

        let names = tables_in_schema(DEFAULT_UNDOTTED_SCHEMA);
        assert!(names.contains(&"test_metrics".to_string()), "got: {names:?}");

        let lazy = read_lazy_from_mmap(DEFAULT_UNDOTTED_SCHEMA, "test_metrics");
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

    #[test]
    fn dotted_schema_isolated_from_memtable_list() {
        let _lock = PROBING_DATA_DIR_LOCK.lock().unwrap();
        use probing_memtable::discover::ExposedTable;

        let tmp = tempfile::tempdir().unwrap();
        let orig = std::env::var("PROBING_DATA_DIR").ok();
        std::env::set_var("PROBING_DATA_DIR", tmp.path());

        let schema = MtSchema::new()
            .col("ts", DType::I64)
            .col("msg", DType::Str);
        let dotted = mmap_filename_for("acme", "metrics_demo");
        let mut ring = ExposedTable::create(&dotted, &schema, 4096, 2).unwrap();
        {
            let mut w = ring.writer();
            w.push_row(&[Value::I64(1), Value::Str("x")]);
        }

        let mem_names = tables_in_schema(DEFAULT_UNDOTTED_SCHEMA);
        assert!(
            !mem_names.contains(&"metrics_demo".to_string()),
            "dotted file must not appear as memtable table: {mem_names:?}"
        );

        let acme_names = tables_in_schema("acme");
        assert!(
            acme_names.contains(&"metrics_demo".to_string()),
            "got: {acme_names:?}"
        );

        let lazy = read_lazy_from_mmap("acme", "metrics_demo");
        assert_eq!(lazy.data.len(), 1);
        assert_eq!(lazy.data[0].num_rows(), 1);

        drop(ring);
        match orig {
            Some(v) => std::env::set_var("PROBING_DATA_DIR", v),
            None => std::env::remove_var("PROBING_DATA_DIR"),
        }
    }

}
