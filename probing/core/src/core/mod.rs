mod arrow_convert;
pub mod cluster;
pub mod cluster_model;
mod engine;
mod error;
pub mod probe_extension;
pub mod memtable_sql;
mod data_source;
mod plugin_advanced;

pub use engine::Engine;
pub use engine::EngineBuilder;
pub use data_source::ProbeDataSource;
pub use data_source::ProbeDataSourceKind;

pub use error::EngineError;
pub use error::Result;

pub use data_source::CustomNamespace;
pub use data_source::CustomNamespaceDataSource;
pub use data_source::CustomTable;
pub use data_source::LazyTableSource;
pub use data_source::NamespaceProbeDataSource;
pub use data_source::TableProbeDataSource;
pub use plugin_advanced::PluginAdvancedTable;

pub use memtable_sql::MemTableProbeExtension;
pub use memtable_sql::UnifiedMemtableProbeDataSource;

pub use probe_extension::ProbeExtensionCall;
pub use probe_extension::ProbeExtension;
pub use probe_extension::ProbeExtensionManager;
pub use probe_extension::ProbeExtensionOption;
pub use probe_extension::Maybe;

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
pub use datafusion::common::error::DataFusionError;
pub use datafusion::config::CatalogOptions;

// pub static ENGINE_RUNTIME: Lazy<tokio::runtime::Runtime> = Lazy::new(|| {
//     tokio::runtime::Builder::new_multi_thread()
//         .worker_threads(4)
//         .enable_all()
//         .build()
//         .unwrap()
// });

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
}
