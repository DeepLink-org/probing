//! In-process registry for table/column documentation attached to [`Schema`].
//!
//! Docs are **not** persisted in mmap headers; they live only in Rust (or are
//! registered from Python) and are consumed by the probing Engine semantic catalog.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use crate::schema::Schema;

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

    let mut reg = registry().lock().expect("table doc registry lock");
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
    let reg = registry().lock().expect("table doc registry lock");
    let mut rows: Vec<TableDocs> = reg.values().cloned().collect();
    rows.sort_by(|a, b| (&a.table_schema, &a.table_name).cmp(&(&b.table_schema, &b.table_name)));
    rows
}

/// Register column docs without a full schema (e.g. Python `@table` before first append).
pub fn register_column_docs(
    table_schema: &str,
    table_name: &str,
    table_doc: Option<&str>,
    columns: &[(String, String)],
) {
    let key = qualified_key(table_schema, table_name);
    let mut reg = registry().lock().expect("table doc registry lock");
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
