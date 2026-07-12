use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use arrow::compute::concat_batches;
use datafusion::catalog::MemoryCatalogProvider;
use datafusion::catalog::MemorySchemaProvider;
use datafusion::config::ConfigExtension;
use datafusion::error::DataFusionError;
use datafusion::error::Result;
use datafusion::execution::SessionState;
use datafusion::prelude::{DataFrame, SessionConfig, SessionContext};

use super::arrow_convert::{arrow_array_to_seq, empty_seq_for_data_type};
use super::probe_extension::ProbeExtension;
use super::probe_extension::ProbeExtensionManager;

use super::data_source::{ProbeDataSource, ProbeDataSourceKind};
use super::federation;
use super::metadata_rewrite;
use super::semantic_catalog;

/// Core query engine for the Probing system
///
/// The Engine provides SQL query capabilities over various data sources
/// through a plugin system. It wraps DataFusion's SessionContext and manages
/// the lifecycle of registered plugins.
///
/// # Data Organization
///
/// Data in the engine is organized hierarchically:
/// - Namespace (provided by plugins)
///   - Table (provided by plugins)
///
/// Note: Internally, namespaces are mapped to DataFusion schemas within a default catalog.
///
/// # Usage Example
///
/// ```
/// // Create a new engine using the builder pattern
/// let engine = probing_core::core::Engine::builder()
///     .with_default_namespace("example_namespace")
///     .build().unwrap();
///
/// // Execute a SQL query
/// let result = engine.async_query("SELECT * FROM information_schema.tables").await?;
/// ```
pub struct Engine {
    /// DataFusion session context for executing SQL queries
    pub context: SessionContext,
    /// Registry of enabled plugins, mapped by their fully qualified names
    data_sources: RwLock<HashMap<String, Arc<dyn ProbeDataSource + Sync + Send>>>,
}

impl Clone for Engine {
    fn clone(&self) -> Self {
        let plugins_clone = match self.data_sources.try_read() {
            Ok(guard) => guard.clone(),
            Err(_) => {
                log::warn!("Engine::clone: data_sources contended; blocking for read");
                if let Ok(handle) = tokio::runtime::Handle::try_current() {
                    tokio::task::block_in_place(|| {
                        handle.block_on(async { self.data_sources.read().await.clone() })
                    })
                } else {
                    crate::runtime::CORE_RUNTIME
                        .block_on(async { self.data_sources.read().await.clone() })
                }
            }
        };
        Self {
            context: self.context.clone(),
            data_sources: RwLock::new(plugins_clone),
        }
    }
}

impl Default for Engine {
    /// Creates a new Engine instance with default configuration
    ///
    /// The default engine:
    /// - Enables the information schema for metadata queries
    /// - Sets "probe" as both the default namespace
    /// - Has no plugins registered initially
    fn default() -> Self {
        let config = SessionConfig::default()
            .with_information_schema(true)
            .with_default_catalog_and_schema("probe", "probe");
        Engine {
            context: SessionContext::new_with_config(config),
            data_sources: Default::default(),
        }
    }
}

impl Engine {
    pub fn builder() -> EngineBuilder {
        EngineBuilder::new()
    }

    pub fn register_extension_options<T: ConfigExtension>(&self, extension: T) {
        self.context
            .state()
            .config_mut()
            .options_mut()
            .extensions
            .insert(extension);
    }

    pub async fn sql(&self, query: &str) -> Result<DataFrame> {
        self.context.sql(query).await
    }

    pub async fn async_query<T: Into<String>>(
        &self,
        query: T,
    ) -> Result<Option<probing_proto::prelude::DataFrame>> {
        let original: String = query.into();
        let capped = federation::ensure_global_scan_limit(&original);
        if let Some(df) = federation::try_execute_aggregate_pushdown(self, &capped).await? {
            federation::check_fanout_strict()?;
            return Ok(Some(df));
        }
        let default_schema = self.default_namespace();
        let query: String =
            metadata_rewrite::prepare_metadata_query(&capped, &default_schema).unwrap_or(capped);
        let query: String = federation::prepare_global_query(&query);
        let df = self.sql(query.as_str()).await?;
        let schema = df.schema().clone();
        let batches = df.collect().await?;
        federation::check_fanout_strict()?;
        if batches.is_empty() {
            let names = schema
                .fields()
                .iter()
                .map(|f| f.name().clone())
                .collect::<Vec<_>>();
            let columns = schema
                .fields()
                .iter()
                .map(|f| empty_seq_for_data_type(f.data_type()))
                .collect::<Vec<_>>();
            return Ok(Some(probing_proto::prelude::DataFrame::new(names, columns)));
        }
        let batch = concat_batches(&batches[0].schema(), batches.iter())?;
        federation::cap_materialized_rows(&original, batch.num_rows())?;

        let names = batch
            .schema()
            .fields()
            .iter()
            .map(|x| x.name().clone())
            .collect::<Vec<_>>();
        let columns = batch
            .columns()
            .iter()
            .map(arrow_array_to_seq)
            .collect::<Vec<_>>();
        Ok(Some(probing_proto::prelude::DataFrame::new(names, columns)))
    }

    /// Get default namespace from configuration
    pub fn default_namespace(&self) -> String {
        self.context
            .state()
            .config()
            .options()
            .catalog
            .default_schema
            .clone()
    }

    pub async fn enable(&self, data_source: Arc<dyn ProbeDataSource + Sync + Send>) -> Result<()> {
        let namespace = data_source.namespace();

        let catalog = if let Some(catalog) = self.context.catalog("probe") {
            catalog
        } else {
            self.context
                .register_catalog("probe", Arc::new(MemoryCatalogProvider::new()));
            self.context
                .catalog("probe")
                .ok_or_else(|| DataFusionError::Internal("no catalog `probe`".to_string()))?
        };

        if data_source.kind() == ProbeDataSourceKind::Namespace {
            let state: SessionState = self.context.state();
            data_source.register_namespace(catalog.clone(), &state)?;
            if let Some(wrapper) = data_source.provide_catalog(catalog) {
                self.context.register_catalog("probe", wrapper);
            }
            let mut maps = self.data_sources.write().await;
            maps.insert(format!("probe.{namespace}"), data_source);
        } else if data_source.kind() == ProbeDataSourceKind::Table {
            // In DataFusion, schemas are used to implement namespaces
            if catalog.schema(namespace.as_str()).is_none() {
                catalog
                    .register_schema(namespace.as_str(), Arc::new(MemorySchemaProvider::new()))?;
            }
            let schema = catalog.schema(namespace.as_str()).ok_or_else(|| {
                DataFusionError::Internal(format!("namespace `{namespace}` not found"))
            })?;
            let state: SessionState = self.context.state();
            data_source.register_table(schema, &state)?;
            let mut maps = self.data_sources.write().await;
            maps.insert(
                format!("probe.{}.{}", namespace, data_source.name()),
                data_source,
            );
        }
        Ok(())
    }
}

// Define the EngineBuilder struct
pub struct EngineBuilder {
    config: SessionConfig,
    default_namespace: Option<String>,
    data_sources: Vec<Arc<dyn ProbeDataSource + Sync + Send>>,
    probe_extensions: HashMap<String, Arc<tokio::sync::Mutex<dyn ProbeExtension + Send + Sync>>>,
}

impl EngineBuilder {
    // Create a new EngineBuilder with default settings
    pub fn new() -> Self {
        EngineBuilder {
            config: SessionConfig::default(),
            default_namespace: None,
            data_sources: Vec::new(),
            probe_extensions: Default::default(),
        }
    }

    // Set the default catalog and schema
    pub fn with_default_namespace(mut self, namespace: &str) -> Self {
        self.default_namespace = Some(namespace.to_string());
        self
    }

    // Add a plugin to the builder
    pub fn with_data_source(mut self, plugin: Arc<dyn ProbeDataSource + Sync + Send>) -> Self {
        self.data_sources.push(plugin);
        self
    }

    pub fn with_extension<T>(mut self, ext: T) -> Self
    where
        T: ProbeExtension + Send + Sync + 'static,
    {
        let name = ext.name();
        let ext = Arc::new(tokio::sync::Mutex::new(ext));

        self.probe_extensions.insert(name, ext);
        self
    }

    // Build the Engine with the specified configurations
    pub async fn build(mut self) -> Result<Engine> {
        let mut eem = ProbeExtensionManager;
        for (name, extension) in self.probe_extensions.iter() {
            eem.register(name.clone(), extension.clone()).await;
        }
        self.config.options_mut().extensions.insert(eem);
        if let Some(namespace) = self.default_namespace {
            self.config = self
                .config
                .with_default_catalog_and_schema("probe", &namespace);
        } else {
            self.config = self
                .config
                .with_default_catalog_and_schema("probe", "probe");
        }
        self.config = self.config.with_information_schema(true);

        let context = SessionContext::new_with_config(self.config);
        let engine = Engine {
            context,
            data_sources: Default::default(),
        };
        for data_source in self.data_sources {
            engine.enable(data_source).await?;
        }
        semantic_catalog::install_semantic_catalog(&engine.context)?;
        federation::install_global_catalog(&engine.context)?;

        Ok(engine)
    }
}

impl Default for EngineBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use crate::core::{ProbeExtension, ProbeExtensionCall};

    use super::*;
    use arrow::array::{Int32Array, StringArray};
    use arrow::datatypes::{DataType, Field, Schema, SchemaRef};
    use arrow::record_batch::RecordBatch;
    use datafusion::catalog::memory::{DataSourceExec, MemorySourceConfig};
    use datafusion::catalog::{
        CatalogProvider, MemorySchemaProvider, SchemaProvider, TableProvider,
    };
    use datafusion::execution::context::SessionState;
    use datafusion::logical_expr::{Expr, TableType};
    use datafusion::physical_plan::ExecutionPlan;
    use probing_proto::prelude::Seq;
    use std::sync::Arc;

    #[derive(Debug, Clone)]
    struct TestTableProbeDataSource {
        schema: SchemaRef,
        batches: Vec<RecordBatch>,
    }

    impl Default for TestTableProbeDataSource {
        fn default() -> Self {
            let schema = Arc::new(Schema::new(vec![
                Field::new("id", DataType::Int32, false),
                Field::new("name", DataType::Utf8, false),
            ]));

            let id_array = Int32Array::from(vec![1, 2, 3]);
            let name_array = StringArray::from(vec!["a", "b", "c"]);

            let batch = RecordBatch::try_new(
                schema.clone(),
                vec![Arc::new(id_array), Arc::new(name_array)],
            )
            .unwrap();
            Self {
                schema,
                batches: vec![batch],
            }
        }
    }

    impl ProbeDataSource for TestTableProbeDataSource {
        fn name(&self) -> String {
            "test_table".to_string()
        }

        fn kind(&self) -> ProbeDataSourceKind {
            ProbeDataSourceKind::Table
        }

        fn namespace(&self) -> String {
            "test_namespace".to_string()
        }

        fn register_table(
            &self,
            schema_provider: Arc<dyn SchemaProvider>,
            _state: &SessionState,
        ) -> Result<()> {
            schema_provider.register_table(self.name(), Arc::new(self.clone()))?;
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl TableProvider for TestTableProbeDataSource {
        fn schema(&self) -> SchemaRef {
            self.schema.clone()
        }

        fn table_type(&self) -> TableType {
            TableType::Base
        }

        async fn scan(
            &self,
            _ctx: &dyn datafusion::catalog::Session,
            projection: Option<&Vec<usize>>,
            _filters: &[Expr],
            _limit: Option<usize>,
        ) -> Result<Arc<dyn ExecutionPlan>> {
            let srccfg = MemorySourceConfig::try_new(
                std::slice::from_ref(&self.batches),
                self.schema.clone(),
                projection.cloned(),
            )?;
            let exec = DataSourceExec::new(Arc::new(srccfg));

            Ok(Arc::new(exec))
        }
    }

    #[derive(Default)]
    struct TestNamespaceProbeDataSource {}

    impl ProbeDataSource for TestNamespaceProbeDataSource {
        fn name(&self) -> String {
            "test_namespace".to_string()
        }

        fn kind(&self) -> ProbeDataSourceKind {
            ProbeDataSourceKind::Namespace
        }

        fn namespace(&self) -> String {
            "test_namespace".to_string()
        }
    }

    #[tokio::test]
    async fn test_engine_builder() {
        let engine = Engine::builder().build().await.unwrap();
        assert_eq!(engine.default_namespace(), "probe");

        let engine = Engine::builder()
            .with_default_namespace("test_namespace")
            .build()
            .await
            .unwrap();
        assert_eq!(engine.default_namespace(), "test_namespace");
    }

    #[tokio::test]
    async fn test_plugin_with_data() -> Result<()> {
        let engine = Engine::builder().build().await?;

        let plugin = Arc::new(TestTableProbeDataSource::default());
        engine.enable(plugin).await?;

        let result = engine
            .async_query("SELECT * FROM test_namespace.test_table")
            .await?
            .unwrap();

        assert_eq!(result.names.len(), 2);
        assert_eq!(result.names[0], "id");
        assert_eq!(result.names[1], "name");

        let result = engine
            .async_query("SELECT * FROM test_namespace.test_table WHERE id > 1")
            .await?
            .unwrap();
        if let Seq::SeqI32(ids) = &result.cols[0] {
            assert_eq!(ids.len(), 2);
            assert!(ids.iter().all(|&id| id > 1));
        }

        Ok(())
    }

    #[tokio::test]
    async fn test_extension_registration() {
        #[derive(Debug)]
        struct TestExtension;

        impl ProbeExtension for TestExtension {
            fn name(&self) -> String {
                "test_extension".to_string()
            }
        }

        impl ProbeExtensionCall for TestExtension {}

        let engine = Engine::builder()
            .with_default_namespace("probe")
            .with_extension(TestExtension)
            .with_data_source(Arc::new(TestTableProbeDataSource::default()))
            .build()
            .await
            .unwrap();

        // Verify the plugin is correctly registered
        let result = engine
            .async_query("SELECT * FROM test_namespace.test_table")
            .await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_some());
    }

    #[tokio::test]
    async fn test_plugin_registration() {
        let engine = Engine::builder().build().await.unwrap();

        let table_plugin = Arc::new(TestTableProbeDataSource::default());
        assert!(engine.enable(table_plugin).await.is_ok());

        let namespace_plugin = Arc::new(TestNamespaceProbeDataSource::default());
        assert!(engine.enable(namespace_plugin).await.is_ok());
    }

    #[tokio::test]
    async fn test_basic_queries() {
        let engine = Engine::builder().build().await.unwrap();

        let result = engine
            .async_query("SELECT 1 as num, 'test' as str")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.names.len(), 2);
        assert_eq!(result.names[0], "num");
        assert_eq!(result.names[1], "str");

        let result = engine.async_query("SELECT 1 WHERE 1=0").await.unwrap();
        let result = result.expect("zero-row queries preserve column schema");
        assert_eq!(result.names, vec!["Int64(1)".to_string()]);
        assert_eq!(result.len(), 0);
    }

    #[tokio::test]
    async fn test_query_error_handling() {
        let engine = Engine::builder().build().await.unwrap();

        let result = engine.async_query("SELECT invalid syntax").await;
        assert!(result.is_err());

        let result = engine.async_query("SELECT * FROM nonexistent_table").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_concurrent_queries() {
        use futures::future::join_all;

        let engine = Engine::builder().build().await.unwrap();
        let queries = ["SELECT 1", "SELECT 2", "SELECT 3"];

        let handles: Vec<_> = queries
            .iter()
            .map(|q| {
                let engine = engine.clone();
                let query = q.to_string();
                tokio::spawn(async move { engine.async_query(query).await })
            })
            .collect();

        let results = join_all(handles).await;
        for result in results {
            assert!(result.unwrap().is_ok());
        }
    }

    #[tokio::test]
    async fn test_data_types() {
        let engine = Engine::builder().build().await.unwrap();

        let query = "
            SELECT
                CAST(1 AS INT) as int_val,
                CAST(2.5 AS FLOAT) as float_val,
                'test' as string_val
        ";

        let result = engine.async_query(query).await.unwrap().unwrap();
        assert_eq!(result.names.len(), 3);

        // testing data types
        assert!(matches!(result.cols[0], Seq::SeqI32(_)));
        assert!(matches!(result.cols[1], Seq::SeqF32(_)));
        assert!(matches!(result.cols[2], Seq::SeqText(_)));
    }

    #[tokio::test]
    async fn test_engine_builder_configuration() {
        let builder = Engine::builder().with_default_namespace("test_namespace");

        // testing default namespace
        let engine = builder.build().await.unwrap();
        assert_eq!(engine.default_namespace(), "test_namespace");

        // testing information schema
        let result = engine.async_query("SHOW TABLES").await;
        assert!(result.is_ok());
    }

    // ========== 复杂查询场景测试 ==========
    // 注意：冗长的测试（如JOIN查询、多命名空间等）已移到 tests/engine_complex_tests.rs

    #[tokio::test]
    async fn test_aggregate_queries() -> Result<()> {
        let engine = Engine::builder().build().await?;
        let plugin = Arc::new(TestTableProbeDataSource::default());
        engine.enable(plugin).await?;

        // Test COUNT
        let result = engine
            .async_query("SELECT COUNT(*) as count FROM test_namespace.test_table")
            .await?
            .unwrap();
        assert_eq!(result.names.len(), 1);
        assert_eq!(result.names[0], "count");
        if let Seq::SeqI64(counts) = &result.cols[0] {
            assert_eq!(counts.len(), 1);
            assert_eq!(counts[0], 3);
        }

        // Test GROUP BY
        let result = engine
            .async_query(
                "SELECT name, COUNT(*) as count FROM test_namespace.test_table GROUP BY name",
            )
            .await?
            .unwrap();
        assert_eq!(result.names.len(), 2);
        assert_eq!(result.names[0], "name");
        assert_eq!(result.names[1], "count");

        Ok(())
    }

    #[tokio::test]
    async fn test_subquery() -> Result<()> {
        let engine = Engine::builder().build().await?;
        let plugin = Arc::new(TestTableProbeDataSource::default());
        engine.enable(plugin).await?;

        // Test scalar subquery
        let result = engine
            .async_query(
                "SELECT id, name, (SELECT MAX(id) FROM test_namespace.test_table) as max_id
                 FROM test_namespace.test_table",
            )
            .await?;
        assert!(result.is_some());

        // Test EXISTS subquery
        let result = engine
            .async_query(
                "SELECT id, name
                 FROM test_namespace.test_table t1
                 WHERE EXISTS (SELECT 1 FROM test_namespace.test_table t2 WHERE t2.id > t1.id)",
            )
            .await?;
        assert!(result.is_some());

        Ok(())
    }

    #[tokio::test]
    async fn test_window_functions() -> Result<()> {
        let engine = Engine::builder().build().await?;
        let plugin = Arc::new(TestTableProbeDataSource::default());
        engine.enable(plugin).await?;

        // Test ROW_NUMBER()
        let result = engine
            .async_query(
                "SELECT id, name, ROW_NUMBER() OVER (ORDER BY id) as row_num
                 FROM test_namespace.test_table",
            )
            .await?;
        assert!(result.is_some());

        // Test RANK()
        let result = engine
            .async_query(
                "SELECT id, name, RANK() OVER (ORDER BY id) as rank
                 FROM test_namespace.test_table",
            )
            .await?;
        assert!(result.is_some());

        Ok(())
    }

    #[tokio::test]
    async fn test_having_clause() -> Result<()> {
        let engine = Engine::builder().build().await?;
        let plugin = Arc::new(TestTableProbeDataSource::default());
        engine.enable(plugin).await?;

        // Test HAVING with GROUP BY
        let result = engine
            .async_query(
                "SELECT name, COUNT(*) as count
                 FROM test_namespace.test_table
                 GROUP BY name
                 HAVING COUNT(*) > 0",
            )
            .await?;
        assert!(result.is_some());

        Ok(())
    }

    // ========== 插件系统测试 ==========
    // 注意：冗长的测试（如多命名空间、并发注册等）已移到 tests/engine_complex_tests.rs

    #[tokio::test]
    async fn test_duplicate_plugin_registration() -> Result<()> {
        let engine = Engine::builder().build().await?;

        // Register the same plugin twice
        let plugin1 = Arc::new(TestTableProbeDataSource::default());
        engine.enable(plugin1.clone()).await?;

        // Registering the same plugin again should either succeed (replace) or fail gracefully
        let result = engine.enable(plugin1).await;
        // The behavior depends on DataFusion's implementation
        // We just verify it doesn't panic
        assert!(result.is_ok() || result.is_err());

        Ok(())
    }

    // ========== 错误处理测试 ==========

    #[tokio::test]
    async fn test_sql_syntax_errors() {
        let engine = Engine::builder().build().await.unwrap();

        // Missing SELECT keyword
        let result = engine.async_query("FROM test_table").await;
        assert!(result.is_err());

        // Invalid SQL syntax
        let result = engine.async_query("SELECT * FROM WHERE id = 1").await;
        assert!(result.is_err());

        // Missing table name
        let result = engine.async_query("SELECT * FROM").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_nonexistent_table() {
        let engine = Engine::builder().build().await.unwrap();

        // Query non-existent table
        let result = engine
            .async_query("SELECT * FROM nonexistent_namespace.nonexistent_table")
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_nonexistent_column() -> Result<()> {
        let engine = Engine::builder().build().await?;
        let plugin = Arc::new(TestTableProbeDataSource::default());
        engine.enable(plugin).await?;

        // Query non-existent column
        let result = engine
            .async_query("SELECT nonexistent_column FROM test_namespace.test_table")
            .await;
        assert!(result.is_err());

        Ok(())
    }

    #[tokio::test]
    async fn test_type_conversion_errors() -> Result<()> {
        let engine = Engine::builder().build().await?;
        let plugin = Arc::new(TestTableProbeDataSource::default());
        engine.enable(plugin).await?;

        // Try to compare incompatible types
        let result = engine
            .async_query("SELECT * FROM test_namespace.test_table WHERE id = 'string'")
            .await;
        // This might succeed with type coercion or fail, depending on DataFusion
        // We just verify it doesn't panic
        assert!(result.is_ok() || result.is_err());

        Ok(())
    }

    #[tokio::test]
    async fn test_invalid_where_clause() -> Result<()> {
        let engine = Engine::builder().build().await?;
        let plugin = Arc::new(TestTableProbeDataSource::default());
        engine.enable(plugin).await?;

        // Invalid WHERE clause
        let result = engine
            .async_query("SELECT * FROM test_namespace.test_table WHERE")
            .await;
        assert!(result.is_err());

        Ok(())
    }

    // ========== 并发查询测试 ==========

    #[tokio::test]
    async fn test_concurrent_queries_with_data() -> Result<()> {
        use futures::future::join_all;

        let engine = Engine::builder().build().await?;
        let plugin = Arc::new(TestTableProbeDataSource::default());
        engine.enable(plugin).await?;

        // Run multiple queries concurrently
        let queries = [
            "SELECT * FROM test_namespace.test_table WHERE id = 1",
            "SELECT * FROM test_namespace.test_table WHERE id = 2",
            "SELECT * FROM test_namespace.test_table WHERE id = 3",
            "SELECT COUNT(*) FROM test_namespace.test_table",
        ];

        let handles: Vec<_> = queries
            .iter()
            .map(|q| {
                let engine = engine.clone();
                let query = q.to_string();
                tokio::spawn(async move { engine.async_query(query).await })
            })
            .collect();

        let results = join_all(handles).await;
        for result in results {
            let query_result = result.unwrap();
            assert!(query_result.is_ok());
        }

        Ok(())
    }

    #[tokio::test]
    async fn test_query_result_isolation() -> Result<()> {
        let engine = Engine::builder().build().await?;
        let plugin = Arc::new(TestTableProbeDataSource::default());
        engine.enable(plugin).await?;

        // Run two queries that should return different results
        let result1 = engine
            .async_query("SELECT * FROM test_namespace.test_table WHERE id = 1")
            .await?
            .unwrap();

        let result2 = engine
            .async_query("SELECT * FROM test_namespace.test_table WHERE id = 2")
            .await?
            .unwrap();

        // Results should be isolated
        if let Seq::SeqI32(ids1) = &result1.cols[0] {
            if let Seq::SeqI32(ids2) = &result2.cols[0] {
                assert_ne!(ids1[0], ids2[0]);
            }
        }

        Ok(())
    }

    // ========== 边界情况测试 ==========
    // 注意：冗长的测试（如空表查询）已移到 tests/engine_complex_tests.rs

    #[tokio::test]
    async fn test_order_by() -> Result<()> {
        let engine = Engine::builder().build().await?;
        let plugin = Arc::new(TestTableProbeDataSource::default());
        engine.enable(plugin).await?;

        // Test ORDER BY
        let result = engine
            .async_query("SELECT * FROM test_namespace.test_table ORDER BY id DESC")
            .await?
            .unwrap();

        // Verify ordering (ids should be in descending order)
        if let Seq::SeqI32(ids) = &result.cols[0] {
            assert_eq!(ids.len(), 3);
            assert!(ids[0] >= ids[1]);
            assert!(ids[1] >= ids[2]);
        }

        Ok(())
    }

    #[tokio::test]
    async fn test_limit_clause() -> Result<()> {
        let engine = Engine::builder().build().await?;
        let plugin = Arc::new(TestTableProbeDataSource::default());
        engine.enable(plugin).await?;

        // Test LIMIT
        let result = engine
            .async_query("SELECT * FROM test_namespace.test_table LIMIT 2")
            .await?
            .unwrap();

        // Should return only 2 rows
        if let Seq::SeqI32(ids) = &result.cols[0] {
            assert!(ids.len() <= 2);
        }

        Ok(())
    }

    #[tokio::test]
    async fn test_where_with_multiple_conditions() -> Result<()> {
        let engine = Engine::builder().build().await?;
        let plugin = Arc::new(TestTableProbeDataSource::default());
        engine.enable(plugin).await?;

        // Test WHERE with AND
        let result = engine
            .async_query("SELECT * FROM test_namespace.test_table WHERE id > 1 AND id < 3")
            .await?
            .unwrap();

        if let Seq::SeqI32(ids) = &result.cols[0] {
            assert_eq!(ids.len(), 1);
            assert_eq!(ids[0], 2);
        }

        // Test WHERE with OR
        let result = engine
            .async_query("SELECT * FROM test_namespace.test_table WHERE id = 1 OR id = 3")
            .await?
            .unwrap();

        if let Seq::SeqI32(ids) = &result.cols[0] {
            assert_eq!(ids.len(), 2);
        }

        Ok(())
    }

    // ── provide_catalog: dynamic schema discovery ──────────────────────

    /// A CatalogProvider wrapper that dynamically returns a schema named
    /// "dynamic_sch" with a single table "my_table", simulating mmap
    /// discovery at query time.
    #[derive(Debug)]
    struct DynCatalog {
        inner: Arc<dyn CatalogProvider>,
        /// Toggled ON after "late" schema appears.
        has_dynamic: std::sync::atomic::AtomicBool,
    }

    impl CatalogProvider for DynCatalog {
        fn schema_names(&self) -> Vec<String> {
            let mut names = self.inner.schema_names();
            if self.has_dynamic.load(std::sync::atomic::Ordering::Relaxed)
                && !names.contains(&"dynamic_sch".to_string())
            {
                names.push("dynamic_sch".to_string());
            }
            names
        }

        fn schema(&self, name: &str) -> Option<Arc<dyn SchemaProvider>> {
            if name == "dynamic_sch" && self.has_dynamic.load(std::sync::atomic::Ordering::Relaxed)
            {
                let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int32, false)]));
                let batch = RecordBatch::try_new(
                    schema.clone(),
                    vec![Arc::new(Int32Array::from(vec![1, 2]))],
                )
                .unwrap();
                let mem = MemorySchemaProvider::new();
                let table = Arc::new(
                    datafusion::datasource::MemTable::try_new(schema, vec![vec![batch]]).unwrap(),
                );
                mem.register_table("late_table".into(), table).unwrap();
                return Some(Arc::new(mem));
            }
            self.inner.schema(name)
        }

        fn register_schema(
            &self,
            name: &str,
            schema: Arc<dyn SchemaProvider>,
        ) -> Result<Option<Arc<dyn SchemaProvider>>> {
            self.inner.register_schema(name, schema)
        }
    }

    /// A Namespace plugin that returns a DynCatalog wrapper via provide_catalog.
    struct DynProbeDataSource {
        catalog: std::sync::Mutex<Option<Arc<DynCatalog>>>,
    }

    impl ProbeDataSource for DynProbeDataSource {
        fn name(&self) -> String {
            "dyn".into()
        }
        fn kind(&self) -> ProbeDataSourceKind {
            ProbeDataSourceKind::Namespace
        }
        fn namespace(&self) -> String {
            "dyn".into()
        }

        fn provide_catalog(
            &self,
            inner: Arc<dyn CatalogProvider>,
        ) -> Option<Arc<dyn CatalogProvider>> {
            let cat = Arc::new(DynCatalog {
                inner,
                has_dynamic: std::sync::atomic::AtomicBool::new(false),
            });
            *self.catalog.lock().unwrap() = Some(cat.clone());
            Some(cat)
        }
    }

    #[tokio::test]
    async fn provide_catalog_enables_dynamic_schema() -> Result<()> {
        let plugin = Arc::new(DynProbeDataSource {
            catalog: std::sync::Mutex::new(None),
        });

        let engine = Engine::builder()
            .with_default_namespace("probe")
            .with_data_source(plugin.clone())
            .build()
            .await?;

        // Before enabling the dynamic flag, "dynamic_sch" must not appear.
        let result = engine
            .async_query(
                "SELECT table_schema, table_name \
                 FROM information_schema.tables \
                 WHERE table_catalog = 'probe' AND table_schema = 'dynamic_sch'",
            )
            .await?;
        let row_count = result.map(|df| df.cols[0].len()).unwrap_or(0);
        assert_eq!(row_count, 0, "dynamic_sch must not exist yet");

        // Flip the flag — simulates mmap files appearing after init.
        plugin
            .catalog
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .has_dynamic
            .store(true, std::sync::atomic::Ordering::Relaxed);

        // Now "dynamic_sch" must be visible in information_schema.tables
        let result = engine
            .async_query(
                "SELECT table_schema, table_name \
                 FROM information_schema.tables \
                 WHERE table_catalog = 'probe' AND table_schema = 'dynamic_sch'",
            )
            .await?;
        let table_count = result.map(|df| df.cols[0].len()).unwrap_or(0);
        assert!(
            table_count > 0,
            "dynamic_sch.late_table must appear in information_schema.tables after flag flip"
        );

        // Also verify it appears in schemata
        let result = engine
            .async_query(
                "SELECT schema_name \
                 FROM information_schema.schemata \
                 WHERE catalog_name = 'probe' AND schema_name = 'dynamic_sch'",
            )
            .await?;
        let schema_count = result.map(|df| df.cols[0].len()).unwrap_or(0);
        assert!(
            schema_count > 0,
            "dynamic_sch must appear in information_schema.schemata after flag flip"
        );

        // Verify the table can actually be queried
        let result = engine
            .async_query("SELECT * FROM dynamic_sch.late_table")
            .await?
            .unwrap();
        if let Seq::SeqI32(ids) = &result.cols[0] {
            assert_eq!(ids.len(), 2);
            assert_eq!(ids[0], 1);
            assert_eq!(ids[1], 2);
        } else {
            panic!("expected SeqI32, got {:?}", result.cols[0]);
        }

        Ok(())
    }
}
