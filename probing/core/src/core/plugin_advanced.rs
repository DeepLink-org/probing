//! Advanced [`TableProvider`] path and **shared pushdown helpers** for in-memory Arrow batches.
//!
//! [`PluginAdvancedTable`] is aimed at internal callers (e.g. mmap memtables). The same filter /
//! limit / stats behaviour is reused by [`super::plugin::TableDataSource`](super::plugin::TableDataSource)
//! and [`super::plugin::LazyTableSource`](super::plugin::LazyTableSource) via [`scan_memory_partitions`]
//! and [`supports_filters_pushdown_for_schema`].

use std::collections::HashSet;
use std::fmt::Debug;
use std::sync::Arc;

use async_trait::async_trait;
use datafusion::arrow::array::Int64Array;
use datafusion::arrow::datatypes::{DataType, Field, Schema, SchemaRef};
use datafusion::arrow::record_batch::{RecordBatch, RecordBatchOptions};
use datafusion::catalog::Session;
use datafusion::common::tree_node::TreeNode;
use datafusion::common::DFSchema;
use datafusion::common::Statistics;
use datafusion::datasource::memory::{DataSourceExec, MemorySourceConfig};
use datafusion::datasource::{TableProvider, TableType};
use datafusion::error::{DataFusionError, Result};
use datafusion::logical_expr::{Expr, TableProviderFilterPushDown};
use datafusion::physical_expr::utils::conjunction;
use datafusion::physical_plan::common::compute_record_batch_statistics;
use datafusion::physical_plan::filter::FilterExecBuilder;
use datafusion::physical_plan::ExecutionPlan;

/// In-memory table: one or more partitions of [`RecordBatch`]es sharing `schema`.
///
/// - Declares **filter push-down** for predicates that pass a conservative structural check
///   (no subqueries, all referenced columns exist on the table schema).
/// - Applies pushed filters in `scan` via [`FilterExec`] on top of [`MemorySourceConfig`].
/// - Applies **`LIMIT` / fetch** on the memory source when there are no pushed filters, and on
///   [`FilterExec`] when filters are present (so limit still applies with pushdown).
/// - Exposes **row / null-count style statistics** via [`TableProvider::statistics`].
#[derive(Debug)]
pub struct PluginAdvancedTable {
    /// Logical table name (for `Debug` / tracing only).
    label: String,
    schema: SchemaRef,
    /// Partition layout expected by [`MemorySourceConfig`].
    partitions: Vec<Vec<RecordBatch>>,
}

impl PluginAdvancedTable {
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Build from a single partition list; validates each batch against `schema`.
    pub fn try_new(
        label: impl Into<String>,
        schema: SchemaRef,
        batches: Vec<RecordBatch>,
    ) -> Result<Self> {
        let label = label.into();
        for b in &batches {
            Self::check_batch_schema(&label, &schema, b)?;
        }
        Ok(Self {
            label,
            schema,
            partitions: vec![batches],
        })
    }

    /// Multi-partition layout (advanced; most callers use [`Self::try_new`]).
    pub fn try_new_partitions(
        label: impl Into<String>,
        schema: SchemaRef,
        partitions: Vec<Vec<RecordBatch>>,
    ) -> Result<Self> {
        let label = label.into();
        for part in &partitions {
            for b in part {
                Self::check_batch_schema(&label, &schema, b)?;
            }
        }
        Ok(Self {
            label,
            schema,
            partitions,
        })
    }

    /// Sentinel for invalid mmap / empty inputs (zero-row, minimal schema).
    pub fn empty_sentinel(label: impl Into<String>) -> Self {
        let label = label.into();
        let schema = Arc::new(Schema::new(vec![Field::new(
            "_empty",
            DataType::Int64,
            true,
        )]));
        let empty = RecordBatch::try_new_with_options(
            Arc::clone(&schema),
            vec![Arc::new(Int64Array::from(Vec::<i64>::new()))],
            &RecordBatchOptions::new().with_row_count(Some(0)),
        )
        .expect("empty batch");
        Self {
            label,
            schema,
            partitions: vec![vec![empty]],
        }
    }

    fn check_batch_schema(label: &str, expected: &SchemaRef, batch: &RecordBatch) -> Result<()> {
        let got = batch.schema();
        if got.as_ref() != expected.as_ref() {
            return Err(DataFusionError::Plan(format!(
                "PluginAdvancedTable {label}: batch schema mismatch (expected {expected}, got {got})"
            )));
        }
        Ok(())
    }
}

/// `true` if `expr` contains constructs we cannot evaluate inside a plain memory scan.
pub(crate) fn has_unsupported_pushdown_subexpr(expr: &Expr) -> bool {
    use datafusion::logical_expr::Expr as E;
    expr.exists(|e| {
        Ok(matches!(
            e,
            E::ScalarSubquery(_)
                | E::Exists { .. }
                | E::InSubquery(_)
                | E::Placeholder(_)
                | E::GroupingSet(_)
                | E::OuterReferenceColumn(_, _)
        ))
    })
    .unwrap_or(true)
}

/// Structural gate for [`TableProvider::supports_filters_pushdown`] without a [`Session`].
pub(crate) fn can_push_filter_exact_for_schema(schema: &SchemaRef, expr: &Expr) -> bool {
    if has_unsupported_pushdown_subexpr(expr) {
        return false;
    }
    let names: HashSet<String> = schema
        .fields()
        .iter()
        .map(|f| f.name().clone())
        .collect();
    for c in expr.column_refs() {
        if !names.contains(c.name()) {
            return false;
        }
    }
    true
}

pub(crate) fn supports_filters_pushdown_for_schema(
    schema: &SchemaRef,
    filters: &[&Expr],
) -> Result<Vec<TableProviderFilterPushDown>> {
    Ok(filters
        .iter()
        .map(|f| {
            if can_push_filter_exact_for_schema(schema, f) {
                TableProviderFilterPushDown::Exact
            } else {
                TableProviderFilterPushDown::Unsupported
            }
        })
        .collect())
}

/// Build a scan plan over in-memory partitions with optional filter + limit pushdown.
pub(crate) async fn scan_memory_partitions(
    state: &dyn Session,
    schema: SchemaRef,
    partitions: &[Vec<RecordBatch>],
    projection: Option<&Vec<usize>>,
    filters: &[Expr],
    limit: Option<usize>,
) -> Result<Arc<dyn ExecutionPlan>> {
    let show_sizes = state.config_options().explain.show_sizes;

    let plan: Arc<dyn ExecutionPlan> = if filters.is_empty() {
        let mem = MemorySourceConfig::try_new(partitions, schema.clone(), projection.cloned())?
            .with_show_sizes(show_sizes)
            .with_limit(limit);
        DataSourceExec::from_data_source(mem)
    } else {
        // Predicates are compiled against the FULL table schema, so the
        // source must scan unprojected; otherwise column indices inside the
        // physical predicate would resolve against the projected batch
        // (e.g. `a > 1` silently evaluating on column `b`). The requested
        // projection is applied by FilterExec on the way out.
        let df_schema = DFSchema::try_from(Arc::clone(&schema))?;
        let mut phys = Vec::new();
        for f in filters {
            phys.push(state.create_physical_expr(f.clone(), &df_schema)?);
        }
        let predicate = conjunction(phys);

        let mem = MemorySourceConfig::try_new(partitions, schema.clone(), None)?
            .with_show_sizes(show_sizes);
        let input: Arc<dyn ExecutionPlan> = DataSourceExec::from_data_source(mem);
        let filt = FilterExecBuilder::new(predicate, input)
            .apply_projection(projection.cloned())?
            .with_fetch(limit)
            .build()?;
        Arc::new(filt)
    };

    Ok(plan)
}

#[async_trait]
impl TableProvider for PluginAdvancedTable {
    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }

    fn table_type(&self) -> TableType {
        TableType::Base
    }

    fn supports_filters_pushdown(
        &self,
        filters: &[&Expr],
    ) -> Result<Vec<TableProviderFilterPushDown>> {
        supports_filters_pushdown_for_schema(&self.schema, filters)
    }

    fn statistics(&self) -> Option<Statistics> {
        Some(compute_record_batch_statistics(
            &self.partitions,
            self.schema.as_ref(),
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
        scan_memory_partitions(
            state,
            self.schema(),
            &self.partitions,
            projection,
            filters,
            limit,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use datafusion::arrow::array::Int32Array;
    use datafusion::common::stats::Precision;
    use datafusion::datasource::TableProvider;
    use datafusion::execution::context::TaskContext;
    use datafusion::logical_expr::expr_fn::{out_ref_col, placeholder};
    use datafusion::logical_expr::TableProviderFilterPushDown;
    use datafusion::physical_plan::collect;
    use datafusion::prelude::{col, lit, SessionContext};
    use std::sync::Arc;

    fn test_schema_id() -> SchemaRef {
        Arc::new(Schema::new(vec![Field::new("id", DataType::Int32, false)]))
    }

    fn batch_ids(schema: &SchemaRef, values: Vec<i32>) -> Result<RecordBatch> {
        RecordBatch::try_new(
            Arc::clone(schema),
            vec![Arc::new(Int32Array::from(values))],
        )
        .map_err(|e| DataFusionError::ArrowError(Box::new(e), None))
    }

    // --- construction ---

    #[test]
    fn try_new_accepts_matching_schema() -> Result<()> {
        let schema = test_schema_id();
        let b = batch_ids(&schema, vec![1, 2])?;
        let t = PluginAdvancedTable::try_new("x", Arc::clone(&schema), vec![b])?;
        assert_eq!(t.label(), "x");
        assert_eq!(t.schema().fields().len(), 1);
        Ok(())
    }

    #[test]
    fn try_new_rejects_schema_mismatch() {
        let expected = test_schema_id();
        let wrong = Arc::new(Schema::new(vec![Field::new(
            "other",
            DataType::Int32,
            false,
        )]));
        let batch = batch_ids(&wrong, vec![1]).unwrap();
        let err = PluginAdvancedTable::try_new("bad", expected, vec![batch]).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("batch schema mismatch"),
            "unexpected error: {msg}"
        );
    }

    #[test]
    fn try_new_partitions_validates_all_batches() {
        let schema = test_schema_id();
        let wrong = Arc::new(Schema::new(vec![Field::new(
            "x",
            DataType::Int32,
            false,
        )]));
        let good = batch_ids(&schema, vec![1]).unwrap();
        let bad = batch_ids(&wrong, vec![2]).unwrap();
        let err = PluginAdvancedTable::try_new_partitions(
            "p",
            Arc::clone(&schema),
            vec![vec![good], vec![bad]],
        )
        .unwrap_err();
        assert!(err.to_string().contains("batch schema mismatch"));
    }

    #[test]
    fn try_new_partitions_succeeds() -> Result<()> {
        let schema = test_schema_id();
        let p0 = batch_ids(&schema, vec![1, 2])?;
        let p1 = batch_ids(&schema, vec![3])?;
        let t = PluginAdvancedTable::try_new_partitions("m", schema.clone(), vec![vec![p0], vec![p1]])?;
        let s = t.statistics().expect("stats");
        assert_eq!(s.num_rows, Precision::Exact(3));
        Ok(())
    }

    #[test]
    fn empty_sentinel_zero_rows_and_schema() {
        let t = PluginAdvancedTable::empty_sentinel("mmap-empty");
        assert_eq!(t.label(), "mmap-empty");
        assert_eq!(t.schema().fields().len(), 1);
        assert_eq!(t.schema().field(0).name(), "_empty");
        let s = t.statistics().expect("stats");
        assert_eq!(s.num_rows, Precision::Exact(0));
    }

    // --- pushdown helpers ---

    #[test]
    fn has_unsupported_detects_placeholder_outer_ref() {
        assert!(has_unsupported_pushdown_subexpr(&placeholder("$1")));
        assert!(has_unsupported_pushdown_subexpr(&out_ref_col(
            DataType::Int32,
            "c"
        )));
        assert!(!has_unsupported_pushdown_subexpr(&col("id")));
        assert!(!has_unsupported_pushdown_subexpr(&col("id").gt(lit(0i32))));
    }

    #[test]
    fn can_push_filter_exact_for_schema_gate() {
        let schema = test_schema_id();
        assert!(can_push_filter_exact_for_schema(
            &schema,
            &col("id").gt(lit(1i32))
        ));
        assert!(!can_push_filter_exact_for_schema(
            &schema,
            &col("missing").gt(lit(1i32))
        ));
        assert!(!can_push_filter_exact_for_schema(
            &schema,
            &placeholder("$1")
        ));
    }

    #[test]
    fn supports_filters_pushdown_for_schema_mixed() -> Result<()> {
        let schema = test_schema_id();
        let f1 = col("id").gt(lit(0i32));
        let f2 = col("nope").eq(lit(1i32));
        let v = supports_filters_pushdown_for_schema(&schema, &[&f1, &f2])?;
        assert_eq!(v.len(), 2);
        assert_eq!(v[0], TableProviderFilterPushDown::Exact);
        assert_eq!(v[1], TableProviderFilterPushDown::Unsupported);
        Ok(())
    }

    // --- scan_memory_partitions ---

    #[tokio::test]
    async fn scan_memory_partitions_limit_without_filter() -> Result<()> {
        let schema = test_schema_id();
        let batch = batch_ids(&schema, vec![10, 20, 30, 40])?;
        let ctx = SessionContext::new();
        let state = ctx.state();
        let plan = scan_memory_partitions(
            &state,
            Arc::clone(&schema),
            &[vec![batch]],
            None,
            &[],
            Some(2),
        )
        .await?;
        let batches = collect(plan, Arc::new(TaskContext::default())).await?;
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].num_rows(), 2);
        let arr = batches[0]
            .column(0)
            .as_any()
            .downcast_ref::<Int32Array>()
            .unwrap();
        assert_eq!(arr.value(0), 10);
        assert_eq!(arr.value(1), 20);
        Ok(())
    }

    #[tokio::test]
    async fn scan_memory_partitions_filter_and_limit() -> Result<()> {
        let schema = test_schema_id();
        let batch = batch_ids(&schema, vec![1, 2, 3, 4, 5])?;
        let filter = col("id").gt(lit(2i32));
        let ctx = SessionContext::new();
        let state = ctx.state();
        let plan = scan_memory_partitions(
            &state,
            Arc::clone(&schema),
            &[vec![batch]],
            None,
            std::slice::from_ref(&filter),
            Some(2),
        )
        .await?;
        let batches = collect(plan, Arc::new(TaskContext::default())).await?;
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].num_rows(), 2);
        let arr = batches[0]
            .column(0)
            .as_any()
            .downcast_ref::<Int32Array>()
            .unwrap();
        assert_eq!(arr.value(0), 3);
        assert_eq!(arr.value(1), 4);
        Ok(())
    }

    #[tokio::test]
    async fn scan_memory_partitions_filter_with_projection_uses_full_schema() -> Result<()> {
        // Regression: predicate column (`a`) is NOT part of the projection.
        // The filter must still evaluate against the full schema instead of
        // resolving indices on the projected batch.
        let schema = Arc::new(Schema::new(vec![
            Field::new("a", DataType::Int64, false),
            Field::new("b", DataType::Int64, false),
        ]));
        let batch = RecordBatch::try_new(
            Arc::clone(&schema),
            vec![
                Arc::new(Int64Array::from(vec![1i64, 2, 3])),
                Arc::new(Int64Array::from(vec![10i64, 20, 30])),
            ],
        )?;
        let filter = col("a").gt(lit(1i64));
        let ctx = SessionContext::new();
        let state = ctx.state();
        let plan = scan_memory_partitions(
            &state,
            Arc::clone(&schema),
            &[vec![batch]],
            Some(&vec![1usize]), // project only `b`
            std::slice::from_ref(&filter),
            None,
        )
        .await?;
        let batches = collect(plan, Arc::new(TaskContext::default())).await?;
        assert_eq!(batches.len(), 1);
        let out = &batches[0];
        assert_eq!(out.num_columns(), 1);
        assert_eq!(out.schema().field(0).name(), "b");
        let arr = out
            .column(0)
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr.value(0), 20);
        assert_eq!(arr.value(1), 30);
        Ok(())
    }

    #[tokio::test]
    async fn scan_memory_partitions_invalid_column_in_filter_errors() -> Result<()> {
        let schema = test_schema_id();
        let batch = batch_ids(&schema, vec![1])?;
        let bad_filter = col("unknown").eq(lit(1i32));
        let ctx = SessionContext::new();
        let state = ctx.state();
        let err = scan_memory_partitions(
            &state,
            schema,
            &[vec![batch]],
            None,
            std::slice::from_ref(&bad_filter),
            None,
        )
        .await
        .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("unknown") || msg.contains("column"),
            "unexpected: {msg}"
        );
        Ok(())
    }

    // --- TableProvider ---

    #[test]
    fn table_provider_as_any_and_table_type() -> Result<()> {
        let schema = test_schema_id();
        let t = PluginAdvancedTable::try_new("t", schema, vec![batch_ids(&test_schema_id(), vec![1])?])?;
        assert!(t.as_any().downcast_ref::<PluginAdvancedTable>().is_some());
        assert_eq!(t.table_type(), TableType::Base);
        Ok(())
    }

    #[tokio::test]
    async fn table_provider_supports_filters_pushdown_delegates() -> Result<()> {
        let schema = test_schema_id();
        let t = PluginAdvancedTable::try_new("t", schema, vec![batch_ids(&test_schema_id(), vec![1])?])?;
        let f = col("id").gt(lit(0i32));
        let v = t.supports_filters_pushdown(&[&f])?;
        assert_eq!(v, vec![TableProviderFilterPushDown::Exact]);
        Ok(())
    }

    #[tokio::test]
    async fn filter_and_limit_pushdown_scan() -> Result<()> {
        let schema = Arc::new(Schema::new(vec![Field::new(
            "id",
            DataType::Int32,
            false,
        )]));
        let batch = RecordBatch::try_new(
            Arc::clone(&schema),
            vec![Arc::new(Int32Array::from(vec![1, 2, 3, 4, 5]))],
        )?;
        let table = Arc::new(PluginAdvancedTable::try_new("t", Arc::clone(&schema), vec![batch])?);
        let ctx = SessionContext::new();
        ctx.register_table("t", table)?;
        let df = ctx.sql("SELECT id FROM t WHERE id > 2 LIMIT 2").await?;
        let batches = df.collect().await?;
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].num_rows(), 2);
        let arr = batches[0]
            .column(0)
            .as_any()
            .downcast_ref::<Int32Array>()
            .unwrap();
        assert_eq!(arr.value(0), 3);
        assert_eq!(arr.value(1), 4);
        Ok(())
    }

    #[tokio::test]
    async fn statistics_reports_row_count() -> Result<()> {
        let schema = Arc::new(Schema::new(vec![Field::new(
            "id",
            DataType::Int32,
            false,
        )]));
        let batch = RecordBatch::try_new(
            Arc::clone(&schema),
            vec![Arc::new(Int32Array::from(vec![10, 20]))],
        )?;
        let table = PluginAdvancedTable::try_new("t", schema, vec![batch])?;
        let s = table.statistics().expect("stats");
        assert_eq!(s.num_rows, Precision::Exact(2));
        Ok(())
    }
}
