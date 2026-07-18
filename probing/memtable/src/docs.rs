//! In-process registry for table/column documentation attached to [`Schema`].
//!
//! Docs are **not** persisted in mmap headers; they live only in Rust (or are
//! registered from Python) and are consumed by the probing Engine semantic catalog.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use crate::schema::{DType, Schema};

/// Documentation for one SQL table (`schema.table`).
#[derive(Debug, Clone, Default)]
pub struct TableDocs {
    pub table_schema: String,
    pub table_name: String,
    pub description: Option<String>,
    pub columns: HashMap<String, String>,
}

static REGISTRY: OnceLock<Mutex<HashMap<String, TableDocs>>> = OnceLock::new();

fn registry() -> &'static Mutex<HashMap<String, TableDocs>> {
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn qualified_key(table_schema: &str, table_name: &str) -> String {
    format!("{table_schema}.{table_name}")
}

/// Infer memtable dtype for a documented extern column from its name alone.
pub fn infer_extern_column_dtype(name: &str) -> DType {
    match name {
        "timestamp" | "time" => DType::I64,
        "duration" | "duration_ms" | "step_duration_sec" | "sample_rate" => DType::F64,
        "trace_id" | "span_id" | "parent_id" | "thread_id" => DType::I64,
        "is_shadow" | "sampled" | "shadow_normal" | "shadow_baseline" => DType::I64,
        "allocated"
        | "max_allocated"
        | "cached"
        | "max_cached"
        | "time_offset"
        | "allocated_delta"
        | "max_allocated_delta" => DType::F64,
        "rank" | "world_size" | "group_rank" | "group_size" | "bytes" | "async_op"
        | "micro_batches" | "micro_step" | "local_step" | "global_step" | "seq" | "role_rank"
        | "role_world_size" | "lineno" | "depth" | "ts" | "used_bytes" | "total_bytes"
        | "mem_used_pct" | "gpu_util_pct" | "rss_kb" | "thread_count" | "cpu_total_pct"
        | "timestamp_ns" => DType::I64,
        "engine_id" | "engine_type" | "metric_name" | "labels" | "framework" | "status"
        | "router_addr" | "metrics_url" | "message" | "error" => DType::Str,
        "metric_value" | "value" => DType::F64,
        _ if name.ends_with("_name")
            || name.ends_with("_type")
            || name.ends_with("_url")
            || name.ends_with("_addr") => DType::Str,
        _ if name.ends_with("_value")
            || name.ends_with("_ratio")
            || name.ends_with("_ms")
            || name.ends_with("_tps")
            || name.ends_with("_bytes")
            || name.ends_with("_seconds") => DType::F64,
        _ if name.starts_with("is_") => DType::I64,
        _ if name.ends_with("_sec") || name.ends_with("_rate") => DType::F64,
        _ if name.ends_with("_id") => DType::I64,
        _ => DType::Str,
    }
}

/// Register table/column docs for a qualified SQL name (`hccl.host_ops`, `python.foo`, …).
pub fn register_qualified(table_schema: &str, table_name: &str, schema: &Schema) {
    let key = qualified_key(table_schema, table_name);
    let mut entry = TableDocs {
        table_schema: table_schema.to_string(),
        table_name: table_name.to_string(),
        description: schema.table_doc.clone(),
        columns: HashMap::new(),
    };
    for col in &schema.cols {
        if let Some(doc) = &col.doc {
            entry.columns.insert(col.name.clone(), doc.clone());
        }
    }

    let mut reg = crate::sync::lock_mutex(registry(), "table doc registry");
    reg.insert(key, entry);
}

/// Register docs from an on-disk mmap basename (`hccl.host_ops` or undotted `metrics`).
pub fn register_from_name(name: &str, schema: &Schema) {
    if let Some((table_schema, table_name)) = name.split_once('.') {
        register_qualified(table_schema, table_name, schema);
    } else {
        register_qualified("memtable", name, schema);
    }
}

/// Snapshot all registered docs (sorted by qualified name).
pub fn snapshot() -> Vec<TableDocs> {
    let reg = crate::sync::lock_mutex(registry(), "table doc registry");
    let mut rows: Vec<TableDocs> = reg.values().cloned().collect();
    rows.sort_by(|a, b| (&a.table_schema, &a.table_name).cmp(&(&b.table_schema, &b.table_name)));
    rows
}

/// Column names registered for `schema.table` (sorted, deduplicated).
pub fn registered_column_names(table_schema: &str, table_name: &str) -> Vec<String> {
    let key = qualified_key(table_schema, table_name);
    let reg = crate::sync::lock_mutex(registry(), "table doc registry");
    let Some(entry) = reg.get(&key) else {
        return Vec::new();
    };
    let mut cols: Vec<String> = entry.columns.keys().cloned().collect();
    cols.sort();
    cols.dedup();
    cols
}

/// Register column docs without a full schema (e.g. Python `@table` before first append).
pub fn register_column_docs(
    table_schema: &str,
    table_name: &str,
    table_doc: Option<&str>,
    columns: &[(String, String)],
) {
    let key = qualified_key(table_schema, table_name);
    let mut reg = crate::sync::lock_mutex(registry(), "table doc registry");
    let entry = reg.entry(key).or_insert_with(|| TableDocs {
        table_schema: table_schema.to_string(),
        table_name: table_name.to_string(),
        description: None,
        columns: HashMap::new(),
    });
    if let Some(doc) = table_doc {
        entry.description = Some(doc.to_string());
    }
    for (col, doc) in columns {
        entry.columns.insert(col.clone(), doc.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DType, Schema};

    fn unique_table(prefix: &str) -> String {
        format!("{prefix}_{}", std::process::id())
    }

    #[test]
    fn register_from_schema_snapshot() {
        let schema =
            Schema::new()
                .table_doc("demo table")
                .col_doc("ts", DType::I64, "timestamp ns");
        register_from_name("demo.events", &schema);
        let rows = snapshot();
        assert!(rows.iter().any(|r| {
            r.table_schema == "demo"
                && r.table_name == "events"
                && r.description.as_deref() == Some("demo table")
                && r.columns.get("ts") == Some(&"timestamp ns".to_string())
        }));
    }

    #[test]
    fn register_undotted_name_uses_memtable_schema() {
        let name = unique_table("metrics_doc");
        let schema =
            Schema::new()
                .table_doc("undotted metrics")
                .col_doc("v", DType::I64, "sample value");
        register_from_name(&name, &schema);
        let rows = snapshot();
        assert!(rows.iter().any(|r| {
            r.table_schema == "memtable"
                && r.table_name == name
                && r.description.as_deref() == Some("undotted metrics")
                && r.columns.get("v") == Some(&"sample value".to_string())
        }));
    }

    #[test]
    fn infer_extern_column_dtype_maps_duration_ms_to_f64() {
        assert_eq!(infer_extern_column_dtype("duration_ms"), DType::F64);
        assert_eq!(infer_extern_column_dtype("step_duration_sec"), DType::F64);
        assert_eq!(infer_extern_column_dtype("sample_rate"), DType::F64);
        assert_eq!(infer_extern_column_dtype("rank"), DType::I64);
        assert_eq!(infer_extern_column_dtype("is_shadow"), DType::I64);
        assert_eq!(infer_extern_column_dtype("sampled"), DType::I64);
        assert_eq!(infer_extern_column_dtype("time"), DType::I64);
        assert_eq!(infer_extern_column_dtype("trace_id"), DType::I64);
        assert_eq!(infer_extern_column_dtype("op"), DType::Str);
        assert_eq!(infer_extern_column_dtype("engine_id"), DType::Str);
        assert_eq!(infer_extern_column_dtype("metric_value"), DType::F64);
        assert_eq!(infer_extern_column_dtype("timestamp_ns"), DType::I64);
    }

    #[test]
    fn registered_column_names_sorted() {
        let table = unique_table("col_names");
        register_column_docs(
            "unittest",
            &table,
            None,
            &[
                ("z".to_string(), "last".to_string()),
                ("a".to_string(), "first".to_string()),
            ],
        );
        assert_eq!(
            registered_column_names("unittest", &table),
            vec!["a".to_string(), "z".to_string()]
        );
        assert!(registered_column_names("unittest", "missing").is_empty());
    }

    #[test]
    fn register_column_docs_merges_into_existing_entry() {
        let table = unique_table("merge_docs");
        register_column_docs(
            "unittest",
            &table,
            Some("initial table doc"),
            &[("a".to_string(), "column a".to_string())],
        );
        register_column_docs(
            "unittest",
            &table,
            Some("updated table doc"),
            &[("b".to_string(), "column b".to_string())],
        );
        let rows = snapshot();
        let row = rows
            .iter()
            .find(|r| r.table_schema == "unittest" && r.table_name == table)
            .expect("merged docs row");
        assert_eq!(row.description.as_deref(), Some("updated table doc"));
        assert_eq!(row.columns.get("a"), Some(&"column a".to_string()));
        assert_eq!(row.columns.get("b"), Some(&"column b".to_string()));
    }

    #[test]
    fn register_from_schema_replaces_prior_entry() {
        let table = unique_table("replace_docs");
        register_from_name(
            &format!("unittest.{table}"),
            &Schema::new()
                .table_doc("old")
                .col_doc("x", DType::I32, "old col"),
        );
        register_from_name(
            &format!("unittest.{table}"),
            &Schema::new()
                .table_doc("new")
                .col_doc("y", DType::I32, "new col"),
        );
        let rows = snapshot();
        let row = rows
            .iter()
            .find(|r| r.table_schema == "unittest" && r.table_name == table)
            .expect("replaced docs row");
        assert_eq!(row.description.as_deref(), Some("new"));
        assert!(!row.columns.contains_key("x"));
        assert_eq!(row.columns.get("y"), Some(&"new col".to_string()));
    }
}
