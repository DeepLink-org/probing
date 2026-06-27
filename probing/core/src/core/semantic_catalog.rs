//! Semantic table/column documentation for Engine `DESCRIBE` / `SHOW CREATE TABLE`.
//!
//! **Primary source:** in-code [`Schema`] docs registered via [`probing_memtable::docs`]
//! (HCCL/NCCL collectors, mmap `ExposedTable::create`, Python `@table`).
//!
//! **Overlay:** `skills/semantic/tables.yaml` supplies agent synonyms/notes/global_name
//! and fills gaps for tables not yet migrated to code-first docs.

use std::collections::HashMap;
use std::sync::Arc;

use arrow::array::{RecordBatch, StringArray};
use arrow::datatypes::{DataType, Field, Schema, SchemaRef};
use datafusion::catalog::{
    CatalogProvider, MemoryCatalogProvider, MemorySchemaProvider, SchemaProvider,
};
use datafusion::error::{DataFusionError, Result};
use datafusion::prelude::SessionContext;
use probing_memtable::docs;
use probing_memtable::infer_extern_column_dtype;
use probing_memtable::DType;
use serde::Deserialize;

use super::plugin_advanced::PluginAdvancedTable;

const TABLES_YAML: &str = include_str!("../../../../skills/semantic/tables.yaml");

pub const DOCS_SCHEMA: &str = "probing";
pub const TABLE_DOCS: &str = "table_docs";
pub const COLUMN_DOCS: &str = "column_docs";

#[derive(Debug, Deserialize)]
struct SemanticCatalogFile {
    tables: HashMap<String, TableEntry>,
}

#[derive(Debug, Deserialize)]
struct TableEntry {
    description: String,
    #[serde(default)]
    synonyms: Vec<String>,
    #[serde(default)]
    key_columns: HashMap<String, String>,
    #[serde(default)]
    notes: Vec<String>,
    #[serde(default)]
    global_name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ParsedSemanticCatalog {
    pub table_rows: Vec<TableDocRow>,
    pub column_rows: Vec<ColumnDocRow>,
}

#[derive(Debug, Clone)]
pub struct TableDocRow {
    pub table_schema: String,
    pub table_name: String,
    pub description: String,
    pub synonyms: String,
    pub notes: String,
    pub global_name: String,
}

#[derive(Debug, Clone)]
pub struct ColumnDocRow {
    pub table_schema: String,
    pub table_name: String,
    pub column_name: String,
    pub description: String,
}

fn table_key(table_schema: &str, table_name: &str) -> (String, String) {
    (table_schema.to_string(), table_name.to_string())
}

fn column_key(table_schema: &str, table_name: &str, column_name: &str) -> (String, String, String) {
    (
        table_schema.to_string(),
        table_name.to_string(),
        column_name.to_string(),
    )
}

/// Register compile-time known collector schemas (HCCL, NCCL, …).
pub fn register_builtin_schema_docs() {
    #[cfg(feature = "builtin-schema-docs")]
    {
        probing_hccl_shim::register_docs();
        probing_nccl_profiler::register_docs();
    }
}

pub fn parse_semantic_catalog_yaml(yaml: &str) -> Result<ParsedSemanticCatalog> {
    let file: SemanticCatalogFile = serde_yaml::from_str(yaml).map_err(|e| {
        DataFusionError::External(format!("failed to parse semantic tables.yaml: {e}").into())
    })?;

    let mut table_rows = Vec::new();
    let mut column_rows = Vec::new();

    for (qualified, entry) in file.tables {
        let Some((table_schema, table_name)) = qualified.split_once('.') else {
            continue;
        };
        table_rows.push(TableDocRow {
            table_schema: table_schema.to_string(),
            table_name: table_name.to_string(),
            description: entry.description,
            synonyms: entry.synonyms.join(", "),
            notes: entry.notes.join("\n"),
            global_name: entry.global_name.unwrap_or_default(),
        });
        for (column_name, description) in entry.key_columns {
            column_rows.push(ColumnDocRow {
                table_schema: table_schema.to_string(),
                table_name: table_name.to_string(),
                column_name,
                description,
            });
        }
    }

    sort_catalog_rows(&mut table_rows, &mut column_rows);
    Ok(ParsedSemanticCatalog {
        table_rows,
        column_rows,
    })
}

fn sort_catalog_rows(table_rows: &mut [TableDocRow], column_rows: &mut [ColumnDocRow]) {
    table_rows
        .sort_by(|a, b| (&a.table_schema, &a.table_name).cmp(&(&b.table_schema, &b.table_name)));
    column_rows.sort_by(|a, b| {
        (&a.table_schema, &a.table_name, &a.column_name).cmp(&(
            &b.table_schema,
            &b.table_name,
            &b.column_name,
        ))
    });
}

/// Merge YAML overlay with the in-code doc registry (registry wins for descriptions).
pub fn build_semantic_catalog() -> Result<ParsedSemanticCatalog> {
    register_builtin_schema_docs();

    let yaml = parse_semantic_catalog_yaml(TABLES_YAML)?;

    let mut table_map: HashMap<(String, String), TableDocRow> = HashMap::new();
    for row in yaml.table_rows {
        table_map.insert(table_key(&row.table_schema, &row.table_name), row);
    }

    let mut column_map: HashMap<(String, String, String), ColumnDocRow> = HashMap::new();
    for row in yaml.column_rows {
        column_map.insert(
            column_key(&row.table_schema, &row.table_name, &row.column_name),
            row,
        );
    }

    for doc in docs::snapshot() {
        let key = table_key(&doc.table_schema, &doc.table_name);
        let entry = table_map.entry(key).or_insert_with(|| TableDocRow {
            table_schema: doc.table_schema.clone(),
            table_name: doc.table_name.clone(),
            description: String::new(),
            synonyms: String::new(),
            notes: String::new(),
            global_name: String::new(),
        });
        if let Some(desc) = &doc.description {
            entry.description = desc.clone();
        }
        for (column_name, description) in &doc.columns {
            column_map.insert(
                column_key(&doc.table_schema, &doc.table_name, column_name),
                ColumnDocRow {
                    table_schema: doc.table_schema.clone(),
                    table_name: doc.table_name.clone(),
                    column_name: column_name.clone(),
                    description: description.clone(),
                },
            );
        }
    }

    let mut table_rows: Vec<TableDocRow> = table_map.into_values().collect();
    let mut column_rows: Vec<ColumnDocRow> = column_map.into_values().collect();
    sort_catalog_rows(&mut table_rows, &mut column_rows);

    Ok(ParsedSemanticCatalog {
        table_rows,
        column_rows,
    })
}

use std::sync::LazyLock;

use datafusion::catalog::TableProvider;

static SEMANTIC_COLUMN_INDEX: LazyLock<HashMap<(String, String), Vec<String>>> =
    LazyLock::new(|| {
        let yaml = match parse_semantic_catalog_yaml(TABLES_YAML) {
            Ok(yaml) => yaml,
            Err(e) => {
                log::error!("skills/semantic/tables.yaml failed to parse: {e}");
                return HashMap::new();
            }
        };
        let mut map: HashMap<(String, String), Vec<String>> = HashMap::new();
        for row in yaml.column_rows {
            map.entry((row.table_schema, row.table_name))
                .or_default()
                .push(row.column_name);
        }
        for cols in map.values_mut() {
            cols.sort();
            cols.dedup();
        }
        map
    });

/// Merge YAML `key_columns` with runtime [`docs`] registry (e.g. Python `@table`).
fn python_extern_table_column_names(table_name: &str) -> Option<Vec<String>> {
    let mut cols = SEMANTIC_COLUMN_INDEX
        .get(&("python".into(), table_name.into()))
        .cloned()
        .unwrap_or_default();
    for col in docs::registered_column_names("python", table_name) {
        if !cols.iter().any(|c| c == &col) {
            cols.push(col);
        }
    }
    if cols.is_empty() {
        None
    } else {
        cols.sort();
        cols.dedup();
        Some(cols)
    }
}

/// Whether `python.<name>` is a documented extern mmap table (not live `backtrace`).
pub fn known_python_extern_table(table_name: &str) -> bool {
    table_name != "backtrace" && python_extern_table_column_names(table_name).is_some()
}

/// Infer Arrow type for a documented `python.*` extern column (mmap placeholder or empty table).
pub fn infer_python_extern_arrow_dtype(name: &str) -> DataType {
    match infer_extern_column_dtype(name) {
        DType::I64 | DType::I32 | DType::U32 | DType::U64 => DataType::Int64,
        DType::F64 | DType::F32 => DataType::Float64,
        DType::U8 => DataType::UInt8,
        DType::Str | DType::Bytes => DataType::Utf8,
    }
}

/// True when an on-disk mmap schema disagrees with the semantic catalog for known columns.
pub fn python_extern_schema_mismatch(table_name: &str, actual: &Schema) -> bool {
    let Some(expected) = empty_python_extern_table(table_name) else {
        return false;
    };
    for field in expected.schema().fields() {
        let Ok(actual_field) = actual.field_with_name(field.name()) else {
            continue;
        };
        if actual_field.data_type() != field.data_type() {
            return true;
        }
    }
    false
}

fn infer_semantic_column_type(name: &str) -> DataType {
    infer_python_extern_arrow_dtype(name)
}

/// Zero-row table with schema from the semantic catalog (avoids Python import fallback).
pub fn empty_python_extern_table(table_name: &str) -> Option<Arc<dyn TableProvider>> {
    let cols = python_extern_table_column_names(table_name)?;
    let mut fields = vec![Field::new("timestamp", DataType::Int64, true)];
    for col in &cols {
        fields.push(Field::new(
            col.as_str(),
            infer_semantic_column_type(col),
            true,
        ));
    }
    let schema = Arc::new(Schema::new(fields));
    let label = format!("python.{table_name}");
    PluginAdvancedTable::try_new(label, schema, vec![])
        .ok()
        .map(|t| Arc::new(t) as Arc<dyn TableProvider>)
}

fn table_docs_schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("table_schema", DataType::Utf8, false),
        Field::new("table_name", DataType::Utf8, false),
        Field::new("description", DataType::Utf8, false),
        Field::new("synonyms", DataType::Utf8, false),
        Field::new("notes", DataType::Utf8, false),
        Field::new("global_name", DataType::Utf8, false),
    ]))
}

fn column_docs_schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("table_schema", DataType::Utf8, false),
        Field::new("table_name", DataType::Utf8, false),
        Field::new("column_name", DataType::Utf8, false),
        Field::new("description", DataType::Utf8, false),
    ]))
}

fn table_docs_batch(rows: &[TableDocRow]) -> Result<RecordBatch> {
    let schema = table_docs_schema();
    let table_schema = StringArray::from(
        rows.iter()
            .map(|r| r.table_schema.as_str())
            .collect::<Vec<_>>(),
    );
    let table_name = StringArray::from(
        rows.iter()
            .map(|r| r.table_name.as_str())
            .collect::<Vec<_>>(),
    );
    let description = StringArray::from(
        rows.iter()
            .map(|r| r.description.as_str())
            .collect::<Vec<_>>(),
    );
    let synonyms = StringArray::from(rows.iter().map(|r| r.synonyms.as_str()).collect::<Vec<_>>());
    let notes = StringArray::from(rows.iter().map(|r| r.notes.as_str()).collect::<Vec<_>>());
    let global_name = StringArray::from(
        rows.iter()
            .map(|r| r.global_name.as_str())
            .collect::<Vec<_>>(),
    );
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(table_schema),
            Arc::new(table_name),
            Arc::new(description),
            Arc::new(synonyms),
            Arc::new(notes),
            Arc::new(global_name),
        ],
    )
    .map_err(DataFusionError::from)
}

fn column_docs_batch(rows: &[ColumnDocRow]) -> Result<RecordBatch> {
    let schema = column_docs_schema();
    let table_schema = StringArray::from(
        rows.iter()
            .map(|r| r.table_schema.as_str())
            .collect::<Vec<_>>(),
    );
    let table_name = StringArray::from(
        rows.iter()
            .map(|r| r.table_name.as_str())
            .collect::<Vec<_>>(),
    );
    let column_name = StringArray::from(
        rows.iter()
            .map(|r| r.column_name.as_str())
            .collect::<Vec<_>>(),
    );
    let description = StringArray::from(
        rows.iter()
            .map(|r| r.description.as_str())
            .collect::<Vec<_>>(),
    );
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(table_schema),
            Arc::new(table_name),
            Arc::new(column_name),
            Arc::new(description),
        ],
    )
    .map_err(DataFusionError::from)
}

/// Register `probing.table_docs` and `probing.column_docs` on the `probe` catalog.
pub fn install_semantic_catalog(context: &SessionContext) -> Result<()> {
    let parsed = build_semantic_catalog()?;
    let catalog: Arc<dyn CatalogProvider> = if let Some(catalog) = context.catalog("probe") {
        catalog
    } else {
        let c: Arc<dyn CatalogProvider> = Arc::new(MemoryCatalogProvider::new());
        context.register_catalog("probe", Arc::clone(&c));
        c
    };

    let schema: Arc<dyn SchemaProvider> = if let Some(schema) = catalog.schema(DOCS_SCHEMA) {
        schema
    } else {
        let s: Arc<dyn SchemaProvider> = Arc::new(MemorySchemaProvider::new());
        catalog.register_schema(DOCS_SCHEMA, Arc::clone(&s))?;
        s
    };

    let table_batch = table_docs_batch(&parsed.table_rows)?;
    let column_batch = column_docs_batch(&parsed.column_rows)?;

    schema.register_table(
        TABLE_DOCS.to_string(),
        Arc::new(PluginAdvancedTable::try_new(
            TABLE_DOCS,
            table_docs_schema(),
            vec![table_batch],
        )?),
    )?;
    schema.register_table(
        COLUMN_DOCS.to_string(),
        Arc::new(PluginAdvancedTable::try_new(
            COLUMN_DOCS,
            column_docs_schema(),
            vec![column_batch],
        )?),
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::Int64Array;

    #[test]
    fn parse_embedded_yaml_has_python_tables() {
        let parsed = parse_semantic_catalog_yaml(TABLES_YAML).unwrap();
        assert!(parsed
            .table_rows
            .iter()
            .any(|r| r.table_schema == "python" && r.table_name == "torch_trace"));
    }

    #[test]
    fn build_catalog_prefers_code_docs_for_hccl() {
        let parsed = build_semantic_catalog().unwrap();
        let host_ops = parsed
            .table_rows
            .iter()
            .find(|r| r.table_schema == "hccl" && r.table_name == "host_ops")
            .expect("hccl.host_ops");
        assert!(host_ops.description.contains("MSProf Host API"));
        assert!(parsed.column_rows.iter().any(|r| {
            r.table_schema == "hccl"
                && r.table_name == "host_ops"
                && r.column_name == "event_class"
                && r.description.contains("host_hccl_op")
        }));
    }

    #[test]
    fn build_catalog_keeps_yaml_synonyms_for_hccl() {
        let parsed = build_semantic_catalog().unwrap();
        let host_ops = parsed
            .table_rows
            .iter()
            .find(|r| r.table_schema == "hccl" && r.table_name == "host_ops")
            .expect("hccl.host_ops");
        assert!(
            host_ops.synonyms.contains("MSProf"),
            "yaml synonyms should be preserved: {}",
            host_ops.synonyms
        );
    }

    #[test]
    fn build_catalog_includes_registry_only_table() {
        let table = format!("code_only_{}", std::process::id());
        docs::register_from_name(
            &format!("unittest.{table}"),
            &probing_memtable::Schema::new()
                .table_doc("registry-only table")
                .col_doc("id", probing_memtable::DType::I64, "primary id"),
        );
        let parsed = build_semantic_catalog().unwrap();
        assert!(parsed.table_rows.iter().any(|r| {
            r.table_schema == "unittest"
                && r.table_name == table
                && r.description.contains("registry-only")
        }));
        assert!(parsed.column_rows.iter().any(|r| {
            r.table_schema == "unittest" && r.table_name == table && r.column_name == "id"
        }));
    }

    #[test]
    fn build_catalog_nccl_culprit_column_from_code() {
        let parsed = build_semantic_catalog().unwrap();
        let row = parsed
            .column_rows
            .iter()
            .find(|r| {
                r.table_schema == "nccl"
                    && r.table_name == "proxy_ops"
                    && r.column_name == "send_gpu_wait_ns"
            })
            .expect("nccl.proxy_ops.send_gpu_wait_ns");
        assert!(row.description.contains("Culprit"));
    }

    #[test]
    fn empty_python_extern_table_has_comm_collective_columns() {
        let table = empty_python_extern_table("comm_collective").expect("comm_collective schema");
        assert!(table.schema().field_with_name("rank").is_ok());
        assert!(table.schema().field_with_name("duration_ms").is_ok());
        assert!(table.schema().field_with_name("op").is_ok());
        assert_eq!(
            table
                .schema()
                .field_with_name("duration_ms")
                .unwrap()
                .data_type(),
            &DataType::Float64
        );
    }

    #[test]
    fn python_extern_schema_mismatch_detects_str_duration_ms() {
        use arrow::datatypes::{Field, Schema};
        let wrong = Schema::new(vec![
            Field::new("timestamp", DataType::Int64, true),
            Field::new("duration_ms", DataType::Utf8, true),
            Field::new("op", DataType::Utf8, true),
        ]);
        assert!(python_extern_schema_mismatch("comm_collective", &wrong));
        let right = empty_python_extern_table("comm_collective")
            .expect("comm_collective")
            .schema()
            .as_ref()
            .clone();
        assert!(!python_extern_schema_mismatch("comm_collective", &right));
    }

    #[test]
    fn empty_python_extern_table_from_docs_registry_only() {
        let table = format!("registry_only_{}", std::process::id());
        docs::register_column_docs(
            "python",
            &table,
            Some("custom @table"),
            &[
                ("custom_id".to_string(), "primary id".to_string()),
                ("payload".to_string(), "json blob".to_string()),
            ],
        );
        let provider = empty_python_extern_table(&table).expect("registry-only schema");
        assert!(known_python_extern_table(&table));
        assert!(provider.schema().field_with_name("timestamp").is_ok());
        assert!(provider.schema().field_with_name("custom_id").is_ok());
        assert!(provider.schema().field_with_name("payload").is_ok());
    }

    #[test]
    fn empty_python_extern_table_merges_yaml_and_docs_columns() {
        docs::register_column_docs(
            "python",
            "torch_trace",
            None,
            &[("extra_metric".to_string(), "runtime-only col".to_string())],
        );
        let provider = empty_python_extern_table("torch_trace").expect("torch_trace schema");
        assert!(provider.schema().field_with_name("module").is_ok());
        assert!(provider.schema().field_with_name("extra_metric").is_ok());
    }

    #[test]
    fn build_catalog_yaml_only_python_table_still_present() {
        let parsed = build_semantic_catalog().unwrap();
        assert!(parsed
            .table_rows
            .iter()
            .any(|r| r.table_schema == "python" && r.table_name == "torch_trace"));
        assert!(parsed.column_rows.iter().any(|r| {
            r.table_schema == "python" && r.table_name == "torch_trace" && r.column_name == "module"
        }));
    }

    #[tokio::test]
    async fn install_registers_docs_tables() {
        let ctx = SessionContext::new();
        install_semantic_catalog(&ctx).unwrap();
        let df = ctx
            .sql("SELECT count(*) AS n FROM probe.probing.column_docs")
            .await
            .unwrap();
        let batches = df.collect().await.unwrap();
        let col = batches[0]
            .column(0)
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap();
        assert!(col.value(0) > 0);
    }
}
