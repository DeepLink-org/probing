// 测试辅助工具模块
// 提供创建测试插件的通用功能，减少重复代码

use arrow::array::{Int32Array, StringArray};
use arrow::datatypes::{DataType, Field, Schema, SchemaRef};
use arrow::record_batch::RecordBatch;
use datafusion::catalog::memory::{DataSourceExec, MemorySourceConfig};
use datafusion::catalog::SchemaProvider;
use datafusion::datasource::TableProvider;
use datafusion::execution::context::SessionState;
use datafusion::logical_expr::Expr;
use datafusion::physical_plan::ExecutionPlan;
use probing_core::core::{Plugin, PluginType};
use std::any::Any;
use std::sync::Arc;

/// 通用的测试表插件实现
#[derive(Debug, Clone)]
pub struct GenericTablePlugin {
    pub name: String,
    pub namespace: String,
    pub schema: SchemaRef,
    pub batches: Vec<RecordBatch>,
}

impl GenericTablePlugin {
    /// 创建一个简单的测试表插件
    pub fn new(name: &str, namespace: &str, schema: SchemaRef, batches: Vec<RecordBatch>) -> Self {
        Self {
            name: name.to_string(),
            namespace: namespace.to_string(),
            schema,
            batches,
        }
    }

    /// 创建一个包含id和name列的简单测试表
    pub fn simple_table(name: &str, namespace: &str) -> Self {
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

        Self::new(name, namespace, schema, vec![batch])
    }

    /// 创建一个单列测试表
    pub fn single_column_table(
        name: &str,
        namespace: &str,
        column_name: &str,
        values: Vec<i32>,
    ) -> Self {
        let schema = Arc::new(Schema::new(vec![Field::new(
            column_name,
            DataType::Int32,
            false,
        )]));

        let batch =
            RecordBatch::try_new(schema.clone(), vec![Arc::new(Int32Array::from(values))]).unwrap();

        Self::new(name, namespace, schema, vec![batch])
    }

    /// 创建一个空表
    pub fn empty_table(name: &str, namespace: &str) -> Self {
        let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int32, false)]));

        let empty_array: Int32Array = Int32Array::from(vec![] as Vec<i32>);
        let batch = RecordBatch::try_new(schema.clone(), vec![Arc::new(empty_array)]).unwrap();

        Self::new(name, namespace, schema, vec![batch])
    }
}

impl Plugin for GenericTablePlugin {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn kind(&self) -> PluginType {
        PluginType::Table
    }

    fn namespace(&self) -> String {
        self.namespace.clone()
    }

    fn register_table(
        &self,
        schema_provider: Arc<dyn SchemaProvider>,
        _state: &SessionState,
    ) -> datafusion::error::Result<()> {
        schema_provider.register_table(self.name(), Arc::new(self.clone()))?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl TableProvider for GenericTablePlugin {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn schema(&self) -> SchemaRef {
        self.schema.clone()
    }

    fn table_type(&self) -> datafusion::logical_expr::TableType {
        datafusion::logical_expr::TableType::Base
    }

    async fn scan(
        &self,
        _ctx: &dyn datafusion::catalog::Session,
        projection: Option<&Vec<usize>>,
        _filters: &[Expr],
        _limit: Option<usize>,
    ) -> datafusion::error::Result<Arc<dyn ExecutionPlan>> {
        let srccfg = MemorySourceConfig::try_new(
            std::slice::from_ref(&self.batches),
            self.schema.clone(),
            projection.cloned(),
        )?;
        Ok(Arc::new(DataSourceExec::new(Arc::new(srccfg))))
    }
}
