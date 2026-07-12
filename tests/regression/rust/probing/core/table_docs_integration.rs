//! Integration tests: Schema docs → registry → semantic catalog → Engine SQL.

use std::sync::Arc;

use anyhow::Result;
use probing_core::core::{Engine, UnifiedMemtableProbeDataSource};
use probing_memtable::discover::ExposedTable;
use probing_memtable::{DType, Schema, Value};
use probing_proto::prelude::{DataFrame, Seq};

fn df_col_str(df: &DataFrame, name: &str) -> Vec<String> {
    let idx = df
        .names
        .iter()
        .position(|n| n == name)
        .unwrap_or_else(|| panic!("column {name} missing from {:?}", df.names));
    match &df.cols[idx] {
        Seq::SeqText(values) => values.clone(),
        other => panic!("column {name}: expected SeqText, got {other:?}"),
    }
}

fn with_data_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    std::env::set_var("PROBING_DATA_DIR", dir.path());
    dir
}

async fn engine_with_memtable() -> Result<Engine> {
    Engine::builder()
        .with_data_source(Arc::new(UnifiedMemtableProbeDataSource))
        .build()
        .await
        .map_err(Into::into)
}

#[tokio::test]
async fn mmap_schema_docs_visible_in_semantic_catalog() -> Result<()> {
    let _dir = with_data_dir();
    let table = format!("metrics_{}", std::process::id());
    let qualified = format!("unittest.{table}");
    let schema = Schema::new()
        .table_doc("Integration metrics table")
        .col_doc("latency_ms", DType::F64, "wall-clock latency in ms")
        .col_doc("rank", DType::I32, "torch rank");

    {
        let mut exposed = ExposedTable::create(&qualified, &schema, 4096, 4)?;
        let mut writer = exposed.writer()?;
        writer.push_row(&[Value::F64(12.5), Value::I32(0)]);
    }

    let engine = engine_with_memtable().await?;

    let col_df = engine
        .async_query(format!(
            "SELECT description FROM probe.probing.column_docs \
             WHERE table_schema = 'unittest' AND table_name = '{table}' \
             AND column_name = 'latency_ms'"
        ))
        .await?
        .expect("column doc row");
    let desc = df_col_str(&col_df, "description")
        .into_iter()
        .next()
        .unwrap_or_default();
    assert_eq!(desc, "wall-clock latency in ms");

    let table_df = engine
        .async_query(format!(
            "SELECT description FROM probe.probing.table_docs \
             WHERE table_schema = 'unittest' AND table_name = '{table}'"
        ))
        .await?
        .expect("table doc row");
    let table_desc = df_col_str(&table_df, "description")
        .into_iter()
        .next()
        .unwrap_or_default();
    assert_eq!(table_desc, "Integration metrics table");
    Ok(())
}

#[tokio::test]
async fn describe_static_catalog_table_includes_comment_columns() -> Result<()> {
    let engine = engine_with_memtable().await?;
    let df = engine
        .async_query("DESCRIBE probe.probing.table_docs")
        .await?
        .expect("DESCRIBE rows");

    assert!(
        df.names.iter().any(|n| n == "comment"),
        "DESCRIBE rewrite missing comment: {:?}",
        df.names
    );
    assert!(
        df.names.iter().any(|n| n == "table_comment"),
        "DESCRIBE rewrite missing table_comment: {:?}",
        df.names
    );
    assert!(
        df_col_str(&df, "column_name")
            .iter()
            .any(|n| n == "description"),
        "expected static catalog columns"
    );
    Ok(())
}

#[tokio::test]
async fn catalog_serves_builtin_hccl_and_yaml_synonyms() -> Result<()> {
    probing_hccl_shim::register_docs();
    let engine = engine_with_memtable().await?;

    let col_df = engine
        .async_query(
            "SELECT description FROM probe.probing.column_docs \
             WHERE table_schema = 'hccl' AND table_name = 'tasks' AND column_name = 'task_name'",
        )
        .await?
        .expect("hccl.tasks.task_name doc");
    let desc = df_col_str(&col_df, "description")
        .into_iter()
        .next()
        .unwrap_or_default();
    assert!(
        desc.contains("Memcpy"),
        "expected code-first HCCL column doc, got {desc}"
    );

    let table_df = engine
        .async_query(
            "SELECT description, synonyms FROM probe.probing.table_docs \
             WHERE table_schema = 'hccl' AND table_name = 'host_ops'",
        )
        .await?
        .expect("hccl.host_ops table doc");
    let description = df_col_str(&table_df, "description")
        .into_iter()
        .next()
        .unwrap_or_default();
    let synonyms = df_col_str(&table_df, "synonyms")
        .into_iter()
        .next()
        .unwrap_or_default();
    assert!(description.contains("MSProf Host API"));
    assert!(
        synonyms.contains("MSProf"),
        "yaml synonyms should remain available: {synonyms}"
    );
    Ok(())
}
