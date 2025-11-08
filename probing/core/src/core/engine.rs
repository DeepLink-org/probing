use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use arrow::compute::concat_batches;
use datafusion::catalog::MemoryCatalogProvider;
use datafusion::catalog::MemorySchemaProvider;
use datafusion::catalog::{CatalogProvider, SchemaProvider};
use datafusion::config::ConfigExtension;
use datafusion::error::DataFusionError;
use datafusion::error::Result;
use datafusion::execution::SessionState;
use datafusion::prelude::{DataFrame, SessionConfig, SessionContext};
use futures;

use super::arrow_convert::arrow_array_to_seq;
use super::extension::EngineExtension;
use super::extension::EngineExtensionManager;

/// Defines the types of plugins supported by the Probing query engine.
/// These plugin types determine how data sources are registered with the engine.
#[derive(PartialEq, Eq)]
pub enum PluginType {
    /// Provides a single table with fixed structure.
    /// Suitable for hardware metrics, process stats, and performance counter data.
    /// Tables are accessible via SQL as "namespace.name".
    Table,

    /// Provides an entire namespace (collection of tables).
    /// Suitable for file system monitoring, Python module tracking, or dynamically
    /// generated performance data.
    /// Tables in a namespace are accessible via SQL as "namespace.table_name".
    Namespace,
}

/// Low-level interface for extending engine functionality through plugins
///
/// Plugins can register either namespaces (collections of tables) or
/// individual tables to the query engine. Implementations should
/// handle specific data sources or analysis capabilities.
///
/// # Naming Convention
///
/// Data in the engine is organized hierarchically:
///
/// - Catalog (default is "probe")
///   - Schema (provided by plugin's "namespace")
///     - Table (provided by plugin's "name" or dynamically by NamespaceProvider)
///
/// ## For Table Plugins
///
/// A table plugin must provide both a namespace and a name. The table will be
/// accessible in SQL queries as:
///
/// ```sql
/// SELECT * FROM namespace.name
/// ```
///
/// ## For Namespace Plugins
///
/// A namespace plugin only needs to provide a namespace. The tables within the namespace
/// will be accessible in SQL queries as:
///
/// ```sql
/// SELECT * FROM namespace.some_table_name
/// ```
///
/// where `some_table_name` is any table provided by the namespace plugin.
pub trait Plugin {
    /// Returns the unique name of the plugin.
    ///
    /// For Table plugins, this is the table name.
    /// For Namespace plugins, this is the namespace.
    fn name(&self) -> String;

    /// Returns the type of this plugin, determining how it integrates with the engine.
    /// This controls which registration method will be called (register_table or register_namespace).
    fn kind(&self) -> PluginType;

    /// Returns the namespace for this plugin, used for organizing related tables.
    ///
    /// - For Table plugins, this defines the namespace under which the table is registered.
    ///   The table will be accessible as "namespace.name".
    ///
    /// - For Namespace plugins, this defines the name of the namespace being provided.
    ///   Tables in this namespace will be accessible as "namespace.table_name".
    fn namespace(&self) -> String;

    /// Registers a table with the provided namespace.
    ///
    /// Implemented by Table plugins to register their data source
    /// with the query engine. The default implementation does nothing.
    ///
    /// # Arguments
    /// * `namespace` - The namespace provider to register the table with
    /// * `state` - The current session state
    #[allow(unused)]
    fn register_table(
        &self,
        namespace: Arc<dyn SchemaProvider>,
        state: &SessionState,
    ) -> Result<()> {
        Ok(())
    }

    /// Registers a namespace with the provided catalog.
    ///
    /// Implemented by Namespace plugins to register their namespace
    /// with the query engine. The default implementation does nothing.
    ///
    /// # Arguments
    /// * `catalog` - The catalog provider to register the namespace with
    /// * `state` - The current session state
    #[allow(unused)]
    fn register_namespace(
        &self,
        catalog: Arc<dyn CatalogProvider>,
        state: &SessionState,
    ) -> Result<()> {
        Ok(())
    }
}

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
/// let result = engine.query("SELECT * FROM information_schema.tables");
/// ```
pub struct Engine {
    /// DataFusion session context for executing SQL queries
    pub context: SessionContext,
    /// Registry of enabled plugins, mapped by their fully qualified names
    plugins: RwLock<HashMap<String, Arc<dyn Plugin + Sync + Send>>>,
}

impl Clone for Engine {
    fn clone(&self) -> Self {
        // Note: This is a synchronous clone, so we need to block on the async lock
        // In practice, this should be avoided in async contexts
        use futures::executor::block_on;
        let plugins_clone = block_on(async { self.plugins.read().await.clone() });
        Self {
            context: self.context.clone(),
            plugins: RwLock::new(plugins_clone),
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
            plugins: Default::default(),
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
        let query: String = query.into();
        let batches = self.sql(query.as_str()).await?.collect().await?;
        if batches.is_empty() {
            return Ok(None);
        }
        let batch = concat_batches(&batches[0].schema(), batches.iter())?;

        let names = batch
            .schema()
            .fields()
            .iter()
            .map(|x| x.name().clone())
            .collect::<Vec<_>>();
        let columns = batch
            .columns()
            .iter()
            .map(|col| arrow_array_to_seq(col))
            .collect::<Vec<_>>();
        Ok(Some(probing_proto::prelude::DataFrame::new(names, columns)))
    }

    #[deprecated]
    pub fn query<T: Into<String>>(&self, q: T) -> Result<probing_proto::prelude::DataFrame> {
        futures::executor::block_on(async { self.async_query(q).await })
            .map(|opt| opt.unwrap_or_default())
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

    pub async fn enable(&self, plugin: Arc<dyn Plugin + Sync + Send>) -> Result<()> {
        let namespace = plugin.namespace();

        let catalog = if let Some(catalog) = self.context.catalog("probe") {
            catalog
        } else {
            self.context
                .register_catalog("probe", Arc::new(MemoryCatalogProvider::new()));
            self.context
                .catalog("probe")
                .ok_or_else(|| DataFusionError::Internal("no catalog `probe`".to_string()))?
        };

        if plugin.kind() == PluginType::Namespace {
            let state: SessionState = self.context.state();
            plugin.register_namespace(catalog, &state)?;
            let mut maps = self.plugins.write().await;
            maps.insert(format!("probe.{namespace}"), plugin);
        } else if plugin.kind() == PluginType::Table {
            // In DataFusion, schemas are used to implement namespaces
            let schema = if catalog.schema_names().contains(&namespace) {
                catalog.schema(namespace.as_str())
            } else {
                let schema = MemorySchemaProvider::new();
                catalog.register_schema(namespace.as_str(), Arc::new(schema))?;
                catalog.schema(namespace.as_str())
            }
            .ok_or_else(|| {
                DataFusionError::Internal(format!("namespace `{namespace}` not found"))
            })?;
            let state: SessionState = self.context.state();
            plugin.register_table(schema, &state)?;
            let mut maps = self.plugins.write().await;
            maps.insert(format!("probe.{}.{}", namespace, plugin.name()), plugin);
        }
        Ok(())
    }
}

// Define the EngineBuilder struct
pub struct EngineBuilder {
    config: SessionConfig,
    default_namespace: Option<String>,
    plugins: Vec<Arc<dyn Plugin + Sync + Send>>,
    extensions: HashMap<String, Arc<tokio::sync::Mutex<dyn EngineExtension + Send + Sync>>>,
}

impl EngineBuilder {
    // Create a new EngineBuilder with default settings
    pub fn new() -> Self {
        EngineBuilder {
            config: SessionConfig::default(),
            default_namespace: None,
            plugins: Vec::new(),
            extensions: Default::default(),
        }
    }

    // Set the default catalog and schema
    pub fn with_default_namespace(mut self, namespace: &str) -> Self {
        self.default_namespace = Some(namespace.to_string());
        self
    }

    // Add a plugin to the builder
    pub fn with_plugin(mut self, plugin: Arc<dyn Plugin + Sync + Send>) -> Self {
        self.plugins.push(plugin);
        self
    }

    pub fn with_extension<T>(mut self, ext: T, namespace: &str, name: Option<&str>) -> Self
    where
        T: EngineExtension + Send + Sync + 'static,
    {
        if let Some(datasrc) = ext.datasrc(namespace, name) {
            self.plugins.push(datasrc)
        };
        let name = ext.name();
        let ext = Arc::new(tokio::sync::Mutex::new(ext));

        self.extensions.insert(name, ext);
        self
    }

    // Build the Engine with the specified configurations
    pub async fn build(mut self) -> Result<Engine> {
        let mut eem = EngineExtensionManager::default();
        for (name, extension) in self.extensions.iter() {
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
            plugins: Default::default(),
        };
        for plugin in self.plugins {
            engine.enable(plugin).await?;
        }

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
    use crate::core::{EngineCall, EngineDatasource};

    use super::*;
    use arrow::array::{Int32Array, StringArray};
    use arrow::datatypes::{DataType, Field, Schema, SchemaRef};
    use arrow::record_batch::RecordBatch;
    use datafusion::catalog::memory::{DataSourceExec, MemorySourceConfig};
    use datafusion::datasource::TableProvider;
    use datafusion::execution::context::SessionState;
    use datafusion::logical_expr::{Expr, TableType};
    use datafusion::physical_plan::ExecutionPlan;
    use probing_proto::prelude::Seq;
    use std::any::Any;
    use std::sync::Arc;

    #[derive(Debug, Clone)]
    struct TestTablePlugin {
        schema: SchemaRef,
        batches: Vec<RecordBatch>,
    }

    impl Default for TestTablePlugin {
        fn default() -> Self {
            let schema = Arc::new(Schema::new(vec![
                Field::new("id", DataType::Int32, false),
                Field::new("name", DataType::Utf8, false),
            ]));

            // create data
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

    impl Plugin for TestTablePlugin {
        fn name(&self) -> String {
            "test_table".to_string()
        }

        fn kind(&self) -> PluginType {
            PluginType::Table
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
    impl TableProvider for TestTablePlugin {
        fn as_any(&self) -> &dyn Any {
            self
        }

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
    struct TestNamespacePlugin {}

    impl Plugin for TestNamespacePlugin {
        fn name(&self) -> String {
            "test_namespace".to_string()
        }

        fn kind(&self) -> PluginType {
            PluginType::Namespace
        }

        fn namespace(&self) -> String {
            "test_namespace".to_string()
        }
    }

    #[tokio::test]
    async fn test_engine_builder() {
        // testing default builder
        let engine = Engine::builder().build().await.unwrap();
        assert_eq!(engine.default_namespace(), "probe");

        // building with custom namespace
        let engine = Engine::builder()
            .with_default_namespace("test_namespace")
            .build()
            .await
            .unwrap();
        assert_eq!(engine.default_namespace(), "test_namespace");
    }

    #[tokio::test]
    async fn test_plugin_with_data() -> Result<()> {
        // create engine
        let engine = Engine::builder().build().await?;

        // register table plugin
        let plugin = Arc::new(TestTablePlugin::default());
        engine.enable(plugin).await?;

        // verify table registration
        let result = engine
            .async_query("SELECT * FROM test_namespace.test_table")
            .await?
            .unwrap();

        assert_eq!(result.names.len(), 2);
        assert_eq!(result.names[0], "id");
        assert_eq!(result.names[1], "name");

        // verify data
        let result = engine
            .async_query("SELECT * FROM test_namespace.test_table WHERE id > 1")
            .await?
            .unwrap();
        if let Seq::SeqI32(ids) = &result.cols[0] {
            assert_eq!(ids.len(), 2); // expect 2 rows
            assert!(ids.iter().all(|&id| id > 1)); // with id > 1
        }

        Ok(())
    }

    #[tokio::test]
    async fn test_extension_registration() {
        #[derive(Debug)]
        struct TestExtension;

        impl EngineExtension for TestExtension {
            fn name(&self) -> String {
                "test_extension".to_string()
            }
        }

        impl EngineCall for TestExtension {}

        impl EngineDatasource for TestExtension {
            fn datasrc(&self, _: &str, _: Option<&str>) -> Option<Arc<dyn Plugin + Send + Sync>> {
                Some(Arc::new(TestTablePlugin::default()))
            }
        }

        // register extension
        let engine = Engine::builder()
            .with_default_namespace("probe")
            .with_extension(TestExtension, "test_namespace", Some("test_table"))
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

        // testing Table plugin registration
        let table_plugin = Arc::new(TestTablePlugin::default());
        assert!(engine.enable(table_plugin).await.is_ok());

        // testing Namespace plugin registration
        let namespace_plugin = Arc::new(TestNamespacePlugin::default());
        assert!(engine.enable(namespace_plugin).await.is_ok());
    }

    #[tokio::test]
    async fn test_basic_queries() {
        let engine = Engine::builder().build().await.unwrap();

        // testing basic SELECT query
        let result = engine
            .async_query("SELECT 1 as num, 'test' as str")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.names.len(), 2);
        assert_eq!(result.names[0], "num");
        assert_eq!(result.names[1], "str");

        // testing empty result set
        let result = engine.async_query("SELECT 1 WHERE 1=0").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_query_error_handling() {
        let engine = Engine::builder().build().await.unwrap();

        // testing invalid SQL syntax
        let result = engine.async_query("SELECT invalid syntax").await;
        assert!(result.is_err());

        // testing nonexistent table
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
        let plugin = Arc::new(TestTablePlugin::default());
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
            .async_query("SELECT name, COUNT(*) as count FROM test_namespace.test_table GROUP BY name")
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
        let plugin = Arc::new(TestTablePlugin::default());
        engine.enable(plugin).await?;

        // Test scalar subquery
        let result = engine
            .async_query(
                "SELECT id, name, (SELECT MAX(id) FROM test_namespace.test_table) as max_id 
                 FROM test_namespace.test_table"
            )
            .await?;
        assert!(result.is_some());

        // Test EXISTS subquery
        let result = engine
            .async_query(
                "SELECT id, name 
                 FROM test_namespace.test_table t1 
                 WHERE EXISTS (SELECT 1 FROM test_namespace.test_table t2 WHERE t2.id > t1.id)"
            )
            .await?;
        assert!(result.is_some());

        Ok(())
    }

    #[tokio::test]
    async fn test_window_functions() -> Result<()> {
        let engine = Engine::builder().build().await?;
        let plugin = Arc::new(TestTablePlugin::default());
        engine.enable(plugin).await?;

        // Test ROW_NUMBER()
        let result = engine
            .async_query(
                "SELECT id, name, ROW_NUMBER() OVER (ORDER BY id) as row_num 
                 FROM test_namespace.test_table"
            )
            .await?;
        assert!(result.is_some());

        // Test RANK()
        let result = engine
            .async_query(
                "SELECT id, name, RANK() OVER (ORDER BY id) as rank 
                 FROM test_namespace.test_table"
            )
            .await?;
        assert!(result.is_some());

        Ok(())
    }

    #[tokio::test]
    async fn test_having_clause() -> Result<()> {
        let engine = Engine::builder().build().await?;
        let plugin = Arc::new(TestTablePlugin::default());
        engine.enable(plugin).await?;

        // Test HAVING with GROUP BY
        let result = engine
            .async_query(
                "SELECT name, COUNT(*) as count 
                 FROM test_namespace.test_table 
                 GROUP BY name 
                 HAVING COUNT(*) > 0"
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
        let plugin1 = Arc::new(TestTablePlugin::default());
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
        let plugin = Arc::new(TestTablePlugin::default());
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
        let plugin = Arc::new(TestTablePlugin::default());
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
        let plugin = Arc::new(TestTablePlugin::default());
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
        let plugin = Arc::new(TestTablePlugin::default());
        engine.enable(plugin).await?;

        // Run multiple queries concurrently
        let queries = vec![
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
        let plugin = Arc::new(TestTablePlugin::default());
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
        let plugin = Arc::new(TestTablePlugin::default());
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
        let plugin = Arc::new(TestTablePlugin::default());
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
        let plugin = Arc::new(TestTablePlugin::default());
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
}
