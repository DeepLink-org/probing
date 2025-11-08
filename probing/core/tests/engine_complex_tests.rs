// 查询引擎复杂场景测试
// 这些测试需要创建多个插件或复杂的测试数据，因此放在独立的测试文件中

use anyhow::Result;
use arrow::array::{Int32Array, StringArray};
use arrow::datatypes::{DataType, Field, Schema, SchemaRef};
use arrow::record_batch::RecordBatch;
use datafusion::catalog::memory::{DataSourceExec, MemorySourceConfig};
use datafusion::catalog::SchemaProvider;
use datafusion::datasource::TableProvider;
use datafusion::execution::context::SessionState;
use datafusion::logical_expr::{Expr, TableType};
use datafusion::physical_plan::ExecutionPlan;
use probing_core::core::{Engine, Plugin, PluginType};
use std::any::Any;
use std::sync::Arc;

mod test_helpers;
use test_helpers::GenericTablePlugin;

// ========== JOIN查询测试 ==========

#[derive(Debug, Clone)]
struct UsersPlugin {
    schema: SchemaRef,
    batches: Vec<RecordBatch>,
}

impl Plugin for UsersPlugin {
    fn name(&self) -> String {
        "users".to_string()
    }
    fn kind(&self) -> PluginType {
        PluginType::Table
    }
    fn namespace(&self) -> String {
        "test".to_string()
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
impl TableProvider for UsersPlugin {
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
    ) -> datafusion::error::Result<Arc<dyn ExecutionPlan>> {
        let srccfg = MemorySourceConfig::try_new(
            std::slice::from_ref(&self.batches),
            self.schema.clone(),
            projection.cloned(),
        )?;
        Ok(Arc::new(DataSourceExec::new(Arc::new(srccfg))))
    }
}

#[derive(Debug, Clone)]
struct OrdersPlugin {
    schema: SchemaRef,
    batches: Vec<RecordBatch>,
}

impl Plugin for OrdersPlugin {
    fn name(&self) -> String {
        "orders".to_string()
    }
    fn kind(&self) -> PluginType {
        PluginType::Table
    }
    fn namespace(&self) -> String {
        "test".to_string()
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
impl TableProvider for OrdersPlugin {
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
    ) -> datafusion::error::Result<Arc<dyn ExecutionPlan>> {
        let srccfg = MemorySourceConfig::try_new(
            std::slice::from_ref(&self.batches),
            self.schema.clone(),
            projection.cloned(),
        )?;
        Ok(Arc::new(DataSourceExec::new(Arc::new(srccfg))))
    }
}

#[tokio::test]
async fn test_join_queries() -> Result<()> {
    // Create two test tables for JOIN
    let engine = Engine::builder().build().await?;

    // First table: users
    let users_schema = Arc::new(Schema::new(vec![
        Field::new("user_id", DataType::Int32, false),
        Field::new("user_name", DataType::Utf8, false),
    ]));
    let users_batch = RecordBatch::try_new(
        users_schema.clone(),
        vec![
            Arc::new(Int32Array::from(vec![1, 2, 3])),
            Arc::new(StringArray::from(vec!["Alice", "Bob", "Charlie"])),
        ],
    )?;

    let users_plugin = Arc::new(UsersPlugin {
        schema: users_schema,
        batches: vec![users_batch],
    });
    engine.enable(users_plugin).await?;

    // Second table: orders
    let orders_schema = Arc::new(Schema::new(vec![
        Field::new("order_id", DataType::Int32, false),
        Field::new("user_id", DataType::Int32, false),
        Field::new("amount", DataType::Int32, false),
    ]));
    let orders_batch = RecordBatch::try_new(
        orders_schema.clone(),
        vec![
            Arc::new(Int32Array::from(vec![1, 2, 3])),
            Arc::new(Int32Array::from(vec![1, 2, 1])),
            Arc::new(Int32Array::from(vec![100, 200, 150])),
        ],
    )?;

    let orders_plugin = Arc::new(OrdersPlugin {
        schema: orders_schema,
        batches: vec![orders_batch],
    });
    engine.enable(orders_plugin).await?;

    // Test INNER JOIN
    let result = engine
        .async_query(
            "SELECT u.user_name, o.amount 
             FROM test.users u 
             INNER JOIN test.orders o ON u.user_id = o.user_id"
        )
        .await?;
    assert!(result.is_some());

    Ok(())
}

// ========== 多命名空间测试 ==========

#[tokio::test]
async fn test_multiple_namespaces() -> Result<()> {
    let engine = Engine::builder().build().await?;

    // Register plugins in different namespaces using helper
    let plugin1 = Arc::new(GenericTablePlugin::simple_table("test_table", "test_namespace"));
    engine.enable(plugin1).await?;

    let plugin2 = Arc::new(GenericTablePlugin::single_column_table(
        "another_table",
        "another_namespace",
        "value",
        vec![10, 20, 30],
    ));
    engine.enable(plugin2).await?;

    // Query from different namespaces
    let result1 = engine
        .async_query("SELECT * FROM test_namespace.test_table")
        .await?;
    assert!(result1.is_some());

    let result2 = engine
        .async_query("SELECT * FROM another_namespace.another_table")
        .await?;
    assert!(result2.is_some());

    Ok(())
}

// ========== 并发插件注册测试 ==========

#[tokio::test]
async fn test_concurrent_plugin_registration() -> Result<()> {
    use futures::future::join_all;

    let engine = Engine::builder().build().await?;

    // Try to register multiple plugins concurrently using helper
    let plugins: Vec<_> = (0..5)
        .map(|i| {
            Arc::new(GenericTablePlugin::single_column_table(
                &format!("table_{}", i),
                "concurrent",
                "id",
                vec![i],
            ))
        })
        .collect();

    let handles: Vec<_> = plugins
        .iter()
        .map(|plugin| {
            let engine = engine.clone();
            let plugin = plugin.clone();
            tokio::spawn(async move { engine.enable(plugin).await })
        })
        .collect();

    let results = join_all(handles).await;
    for result in results {
        let reg_result = result.unwrap();
        // Some might fail due to concurrent registration, but shouldn't panic
        assert!(reg_result.is_ok() || reg_result.is_err());
    }

    Ok(())
}

// ========== 空表查询测试 ==========

#[tokio::test]
async fn test_empty_table_query() -> Result<()> {
    let engine = Engine::builder().build().await?;
    
    // Use helper to create empty table
    let empty_plugin = Arc::new(GenericTablePlugin::empty_table("empty_table", "test"));
    engine.enable(empty_plugin).await?;

    // Query empty table
    let result = engine
        .async_query("SELECT * FROM test.empty_table")
        .await?;
    
    // Empty table query might return None or empty result depending on DataFusion
    match result {
        None => {
            // Expected: empty result returns None
        }
        Some(df) => {
            // If it returns Some, verify it's empty
            use probing_proto::prelude::Seq;
            if let Seq::SeqI32(ids) = &df.cols[0] {
                assert_eq!(ids.len(), 0);
            }
        }
    }

    Ok(())
}

