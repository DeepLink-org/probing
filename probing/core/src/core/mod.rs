mod arrow_convert;
pub mod cluster;
pub mod cluster_model;
mod data_source;
mod engine;
mod error;
pub mod federation;
pub mod memtable_sql;
mod metadata_rewrite;
mod plugin_advanced;
pub mod probe_extension;
mod semantic_catalog;

pub use data_source::ProbeDataSource;
pub use data_source::ProbeDataSourceKind;
pub use engine::Engine;
pub use engine::EngineBuilder;

pub use error::EngineError;
pub use error::Result;

pub use probing_memtable::MemtableError;

pub use data_source::CustomNamespace;
pub use data_source::CustomNamespaceDataSource;
pub use data_source::CustomTable;
pub use data_source::LazyTableSource;
pub use data_source::NamespaceProbeDataSource;
pub use data_source::TableProbeDataSource;
pub use plugin_advanced::PluginAdvancedTable;

pub use memtable_sql::MemTableProbeExtension;
pub use memtable_sql::UnifiedMemtableProbeDataSource;

pub use probe_extension::ExtensionConfigSpec;
pub use probe_extension::ExtensionContentType;
pub use probe_extension::ExtensionHttpMethod;
pub use probe_extension::ExtensionRoute;
pub use probe_extension::Maybe;
pub use probe_extension::ProbeExtension;
pub use probe_extension::ProbeExtensionCall;
pub use probe_extension::ProbeExtensionConfig;
pub use probe_extension::ProbeExtensionManager;
pub use probe_extension::ProbeExtensionOption;
pub use probe_extension::ProbeExtensionResponse;

pub use probing_macros::ProbeExtension;

pub use datafusion::arrow::array::ArrayRef;
pub use datafusion::arrow::array::Float32Array;
pub use datafusion::arrow::array::Float64Array;
pub use datafusion::arrow::array::Int32Array;
pub use datafusion::arrow::array::Int64Array;
pub use datafusion::arrow::array::RecordBatch;
pub use datafusion::arrow::array::StringArray;
pub use datafusion::arrow::datatypes::DataType;
pub use datafusion::arrow::datatypes::Field;
pub use datafusion::arrow::datatypes::Schema;
pub use datafusion::arrow::datatypes::SchemaRef;
pub use datafusion::arrow::datatypes::TimeUnit;
pub use datafusion::arrow::util::pretty;
pub use datafusion::catalog::TableProvider;
pub use datafusion::common::error::DataFusionError;
pub use datafusion::config::CatalogOptions;
pub use datafusion::error::Result as DataFusionResult;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn build_engine() {
        let engine = Engine::builder().build().await.unwrap();

        let result = engine.async_query("show tables").await;
        assert!(result.is_ok(), "Should execute SHOW TABLES query");
    }

    #[tokio::test]
    async fn build_engine_with_default_namespace() {
        let engine = Engine::builder()
            .with_default_namespace("test")
            .build()
            .await
            .unwrap();

        assert_eq!(engine.default_namespace(), "test".to_string());
    }

    #[tokio::test]
    async fn execute_basic_queries() {
        let engine = Engine::builder().build().await.unwrap();

        // Test SHOW TABLES
        let show_tables = engine.async_query("show tables").await;
        assert!(show_tables.is_ok(), "SHOW TABLES should succeed");

        // Test SELECT
        let select_query = engine.async_query("SELECT 1 as val").await;
        assert!(select_query.is_ok(), "SELECT should return results");

        // Verify result schema
        let df = select_query.unwrap().unwrap();
        assert_eq!(df.names.len(), 1, "Should have one column");
        assert_eq!(df.names[0], "val", "Column name should match");
        assert!(!df.cols.is_empty(), "Should have data columns");
    }

    #[tokio::test]
    async fn describe_rewrite_includes_comment() {
        let engine = Engine::builder().build().await.unwrap();
        let df = engine
            .async_query("DESCRIBE probing.column_docs")
            .await
            .unwrap()
            .unwrap();
        assert!(
            df.names.iter().any(|n| n == "comment"),
            "DESCRIBE rewrite should expose comment column, got {:?}",
            df.names
        );
        assert!(
            df.names.iter().any(|n| n == "table_comment"),
            "DESCRIBE rewrite should expose table_comment column, got {:?}",
            df.names
        );
    }

    #[tokio::test]
    async fn engine_column_docs_serves_code_first_registry_entry() {
        use probing_proto::prelude::Seq;

        probing_memtable::docs::register_from_name(
            "unittest.core_column_docs",
            &probing_memtable::Schema::new()
                .table_doc("core registry table")
                .col_doc(
                    "value",
                    probing_memtable::DType::I64,
                    "registry value column",
                ),
        );
        let engine = Engine::builder().build().await.unwrap();
        let df = engine
            .async_query(
                "SELECT description FROM probe.probing.column_docs \
                 WHERE table_schema = 'unittest' AND table_name = 'core_column_docs' AND column_name = 'value'",
            )
            .await
            .unwrap()
            .expect("column_docs query should return rows");
        assert_eq!(df.names, vec!["description"]);
        let desc = match &df.cols[0] {
            Seq::SeqText(values) => values.first().cloned().expect("value description row"),
            other => panic!("expected SeqText, got {other:?}"),
        };
        assert!(
            desc.contains("registry value"),
            "expected code-first column doc, got {desc}"
        );
    }

    #[tokio::test]
    async fn engine_table_docs_serves_code_first_registry_entry() {
        use probing_proto::prelude::Seq;

        probing_memtable::docs::register_from_name(
            "unittest.core_table_docs",
            &probing_memtable::Schema::new().table_doc("core registry table"),
        );
        let engine = Engine::builder().build().await.unwrap();
        let df = engine
            .async_query(
                "SELECT description FROM probe.probing.table_docs \
                 WHERE table_schema = 'unittest' AND table_name = 'core_table_docs'",
            )
            .await
            .unwrap()
            .expect("table_docs query should return rows");
        let desc = match &df.cols[0] {
            Seq::SeqText(values) => values
                .first()
                .cloned()
                .expect("core_table_docs description row"),
            other => panic!("expected SeqText, got {other:?}"),
        };
        assert!(
            desc.contains("core registry table"),
            "expected code-first table doc, got {desc}"
        );
    }
}
