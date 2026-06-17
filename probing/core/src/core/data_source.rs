//! Probe data sources: catalog registration adapters for DataFusion.
//!
//! A [`ProbeDataSource`] registers tables or namespaces into the engine catalog.
//! This is distinct from [`super::probe_extension::ProbeExtension`], which provides HTTP
//! handlers, configuration, and side effects. Register data sources with
//! [`EngineBuilder::with_data_source`](super::engine::EngineBuilder::with_data_source).

use std::{fmt::Debug, marker::PhantomData, sync::Arc};

use datafusion::catalog::{CatalogProvider, SchemaProvider, Session, TableProvider};
use datafusion::error::Result;
use datafusion::execution::SessionState;

/// Kind of probe data source: single table or entire namespace.
#[derive(PartialEq, Eq)]
pub enum ProbeDataSourceKind {
    /// Single table registered under a namespace.
    Table,
    /// Namespace (schema) containing dynamically discovered tables.
    Namespace,
}

/// Catalog registration adapter for Probing query engine data sources.
pub trait ProbeDataSource {
    fn name(&self) -> String;
    fn kind(&self) -> ProbeDataSourceKind;
    fn namespace(&self) -> String;

    #[allow(unused)]
    fn register_table(
        &self,
        namespace: Arc<dyn SchemaProvider>,
        state: &SessionState,
    ) -> Result<()> {
        Ok(())
    }

    #[allow(unused)]
    fn register_namespace(
        &self,
        catalog: Arc<dyn CatalogProvider>,
        state: &SessionState,
    ) -> Result<()> {
        Ok(())
    }

    #[allow(unused)]
    fn provide_catalog(&self, inner: Arc<dyn CatalogProvider>) -> Option<Arc<dyn CatalogProvider>> {
        None
    }
}

use arrow::datatypes::{DataType, Field, Schema};
use async_trait::async_trait;
use datafusion::arrow::array::RecordBatch;
use datafusion::arrow::datatypes::SchemaRef;
use datafusion::common::Statistics;
use datafusion::datasource::TableType;
use datafusion::error::DataFusionError;
use datafusion::logical_expr::TableProviderFilterPushDown;
use datafusion::physical_plan::common::compute_record_batch_statistics;
use datafusion::physical_plan::ExecutionPlan;
use datafusion::prelude::Expr;

use super::plugin_advanced::{scan_memory_partitions, supports_filters_pushdown_for_schema};

/// Trait defining a custom table with static/dynamic schema and data
///
/// Implement this to create tables that:
/// - Have a fixed name
/// - Use a predefined schema
///
/// The default [`TableDataSource`] integration applies **conservative** `WHERE` / `LIMIT`
/// pushdown (same rules as [`super::plugin_advanced`](super::plugin_advanced)): simple predicates
/// whose columns all exist on the table may run inside the scan; others stay in a planner `Filter`.
pub trait CustomTable {
    /// Returns the table name (must be constant)
    fn name() -> &'static str;

    /// Returns the table schema
    fn schema() -> SchemaRef;

    /// Provides the data batches
    fn data() -> Vec<RecordBatch>;
}

/// Helper struct that bridges a CustomTable implementation with the Plugin system.
/// Handles registration and integration with DataFusion query engine.
pub struct TableProbeDataSource<T: CustomTable> {
    /// Name of the table as it will be registered
    name: String,

    /// Namespace the table belongs to
    namespace: String,

    /// PhantomData to track the generic parameter T
    data: PhantomData<T>,
}

impl<T: CustomTable> Default for TableProbeDataSource<T> {
    fn default() -> Self {
        Self {
            name: T::name().to_string(),
            namespace: "probe".to_string(),
            data: Default::default(),
        }
    }
}

/// Methods for constructing and working with TableProbeDataSource instances
impl<T: CustomTable + std::default::Default + std::fmt::Debug + Send + Sync + 'static>
    TableProbeDataSource<T>
{
    /// Creates a new TableProbeDataSource with custom name and namespace
    pub fn new<S: Into<String>>(namespace: S, name: S) -> Self {
        Self {
            name: name.into(),
            namespace: namespace.into(),
            data: PhantomData::<T> {},
        }
    }

    /// Factory method that creates a TableProbeDataSource wrapped in an Arc
    /// Returns a trait object that can be used with the plugin system
    pub fn create<S: Into<String>>(
        namespace: S,
        name: S,
    ) -> Arc<dyn ProbeDataSource + Send + Sync> {
        Arc::new(Self::new(namespace, name))
    }
}

/// Implementation of the Plugin trait for TableProbeDataSource
impl<T: CustomTable + Default + Debug + Send + Sync + 'static> ProbeDataSource
    for TableProbeDataSource<T>
{
    fn name(&self) -> String {
        self.name.clone()
    }

    fn kind(&self) -> ProbeDataSourceKind {
        ProbeDataSourceKind::Table
    }

    fn namespace(&self) -> String {
        self.namespace.clone()
    }

    /// Registers this table with the provided schema provider
    /// Links the CustomTable implementation with DataFusion's query engine
    fn register_table(
        &self,
        schema: std::sync::Arc<dyn datafusion::catalog::SchemaProvider>,
        _state: &datafusion::execution::SessionState,
    ) -> datafusion::error::Result<()> {
        schema.register_table(self.name(), Arc::new(TableDataSource::<T>::default()))?;
        Ok(())
    }
}

#[derive(Clone, Default, Debug)]
pub struct TableDataSource<T: CustomTable> {
    data: PhantomData<T>,
}

#[async_trait]
impl<T: CustomTable + Default + Debug + Send + Sync + 'static> TableProvider
    for TableDataSource<T>
{
    fn schema(&self) -> SchemaRef {
        T::schema()
    }

    fn table_type(&self) -> TableType {
        TableType::Base
    }

    fn supports_filters_pushdown(
        &self,
        filters: &[&Expr],
    ) -> Result<Vec<TableProviderFilterPushDown>> {
        supports_filters_pushdown_for_schema(&T::schema(), filters)
    }

    fn statistics(&self) -> Option<Statistics> {
        let partitions = vec![T::data()];
        Some(compute_record_batch_statistics(
            &partitions,
            T::schema().as_ref(),
            None,
        ))
    }

    async fn scan(
        &self,
        state: &dyn Session,
        projection: Option<&Vec<usize>>,
        filters: &[Expr],
        limit: Option<usize>,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        let batches = T::data();
        let partitions = vec![batches];
        scan_memory_partitions(state, T::schema(), &partitions, projection, filters, limit).await
    }
}

/// Eager in-memory table built from pre-materialized [`RecordBatch`]es (e.g. mmap → Arrow).
///
/// Supports the same **conservative** `WHERE` / `LIMIT` pushdown as [`TableDataSource`] via
/// [`super::plugin_advanced::scan_memory_partitions`](super::plugin_advanced::scan_memory_partitions).
#[derive(Default, Debug)]
pub struct LazyTableSource {
    pub name: String,
    pub schema: Option<SchemaRef>,
    pub data: Vec<RecordBatch>,
}

#[async_trait]
impl TableProvider for LazyTableSource {
    fn schema(&self) -> SchemaRef {
        if let Some(schema) = &self.schema {
            return schema.clone();
        }
        SchemaRef::new(Schema::new(vec![Field::new(
            "unknown_fields",
            DataType::Int64,
            false,
        )]))
    }

    fn table_type(&self) -> TableType {
        TableType::Base
    }

    fn supports_filters_pushdown(
        &self,
        filters: &[&Expr],
    ) -> Result<Vec<TableProviderFilterPushDown>> {
        supports_filters_pushdown_for_schema(&self.schema(), filters)
    }

    fn statistics(&self) -> Option<Statistics> {
        if self.data.is_empty() {
            return None;
        }
        let partitions = vec![self.data.clone()];
        Some(compute_record_batch_statistics(
            &partitions,
            self.schema().as_ref(),
            None,
        ))
    }

    async fn scan(
        &self,
        state: &dyn Session,
        projection: Option<&Vec<usize>>,
        filters: &[Expr],
        limit: Option<usize>,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        let data = &self.data;
        if data.is_empty() {
            return Err(DataFusionError::Execution(
                "no data found for lazy table".to_string(),
            ));
        }
        let schema = data[0].schema();
        let partitions = vec![self.data.clone()];
        scan_memory_partitions(state, schema, &partitions, projection, filters, limit).await
    }
}

/// Trait for implementing a custom namespace that can dynamically generate tables
/// Provides a mechanism for on-demand table creation based on name/expression
#[allow(unused)]
#[async_trait]
pub trait CustomNamespace: Sync + Send {
    /// Returns the name of the namespace
    fn name() -> &'static str;

    /// Returns a list of available table names in this namespace
    fn list() -> Vec<String>;

    /// Generates data for a specific table expression.
    /// Override this in namespace implementations; [`Self::make_lazy`] materializes it.
    fn data(expr: &str) -> Vec<RecordBatch> {
        vec![]
    }

    /// Creates a [`LazyTableSource`] by calling [`Self::data`] and inferring schema from batches.
    fn make_lazy(expr: &str) -> Arc<LazyTableSource>
    where
        Self: Sized,
    {
        let data = Self::data(expr);
        let schema = data.first().map(|batch| batch.schema());
        Arc::new(LazyTableSource {
            name: expr.to_string(),
            schema,
            data,
        })
    }

    /// Factory method to create a TableProvider for a specific table expression
    /// Used by the namespace provider to generate tables on demand
    async fn table(expr: String) -> Result<Option<Arc<dyn TableProvider>>>
    where
        Self: Default + Debug + Send + Sync + Sized + 'static,
    {
        let lazy = Self::make_lazy(expr.as_str());
        Ok(Some(lazy))
    }
}

/// Helper struct that bridges a CustomNamespace implementation with the Plugin system
/// Manages registration and integration with DataFusion query engine
pub struct NamespaceProbeDataSource<T: CustomNamespace> {
    /// Namespace the schema belongs to
    namespace: String,

    /// PhantomData to track the generic parameter T
    data: PhantomData<T>,
}

impl<T: CustomNamespace> Default for NamespaceProbeDataSource<T> {
    fn default() -> Self {
        Self {
            namespace: "probe".to_string(),
            data: Default::default(),
        }
    }
}

impl<T: CustomNamespace + std::default::Default + std::fmt::Debug + Send + Sync + 'static>
    NamespaceProbeDataSource<T>
{
    pub fn new<S: Into<String>>(namespace: S) -> Self {
        Self {
            namespace: namespace.into(),
            data: PhantomData::<T> {},
        }
    }

    pub fn create<S: Into<String>>(namespace: S) -> Arc<dyn ProbeDataSource + Send + Sync> {
        Arc::new(Self::new(namespace))
    }
}

impl<T: CustomNamespace + Default + Debug + Send + Sync + 'static> ProbeDataSource
    for NamespaceProbeDataSource<T>
{
    fn name(&self) -> String {
        self.namespace.clone()
    }

    fn kind(&self) -> ProbeDataSourceKind {
        ProbeDataSourceKind::Namespace
    }

    fn namespace(&self) -> String {
        self.namespace.clone()
    }

    #[allow(unused)]
    fn register_namespace(
        &self,
        catalog: Arc<dyn CatalogProvider>,
        state: &SessionState,
    ) -> Result<()> {
        catalog.register_schema(
            self.namespace().as_str(),
            Arc::new(CustomNamespaceDataSource::<T>::default()),
        );
        Ok(())
    }
}

#[derive(Default, Debug)]
pub struct CustomNamespaceDataSource<T: CustomNamespace> {
    data: PhantomData<T>,
}

#[async_trait]
impl<T: CustomNamespace + Default + Debug + Send + Sync + 'static> SchemaProvider
    for CustomNamespaceDataSource<T>
{
    fn table_names(&self) -> Vec<String> {
        T::list()
    }

    async fn table(&self, name: &str) -> Result<Option<Arc<dyn TableProvider>>> {
        T::table(name.to_string()).await
    }

    #[allow(unused)]
    fn register_table(
        &self,
        name: String,
        table: Arc<dyn TableProvider>,
    ) -> Result<Option<Arc<dyn TableProvider>>> {
        Err(datafusion::error::DataFusionError::NotImplemented(
            "unable to create tables".to_string(),
        ))
    }
    #[allow(unused_variables)]
    fn deregister_table(&self, name: &str) -> Result<Option<Arc<dyn TableProvider>>> {
        Err(DataFusionError::NotImplemented(
            "unable to drop tables".to_string(),
        ))
    }
    fn table_exist(&self, _name: &str) -> bool {
        true
    }
}
