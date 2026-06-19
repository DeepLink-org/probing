use std::sync::Arc;

use async_trait::async_trait;
use datafusion::arrow::datatypes::SchemaRef;
use datafusion::catalog::Session;
use datafusion::common::Statistics;
use datafusion::datasource::{TableProvider, TableType};
use datafusion::error::Result;
use datafusion::logical_expr::{Expr, TableProviderFilterPushDown};
use datafusion::physical_plan::coalesce_partitions::CoalescePartitionsExec;
use datafusion::physical_plan::ExecutionPlan;
use super::cluster_executor::{reset_fanout_stats, ProbeClusterExecutor};
use super::convert::{
    cluster_rank_for_endpoint, extend_projection_with_probe_tags, federated_output_schema,
};
use super::federated_scan_exec::FederatedScanExec;
use super::sql_gen::build_remote_table_sql;

/// Ensure federated scans always expose node tag columns (`_addr`, `_rank`, …).
fn federated_scan_projection(
    projection: Option<&Vec<usize>>,
    output_schema: &SchemaRef,
) -> Option<Vec<usize>> {
    match extend_projection_with_probe_tags(projection, output_schema) {
        None => Some((0..output_schema.fields().len()).collect()),
        Some(idxs) => Some(idxs),
    }
}

/// Map a federated-table projection to indices on the local `probe` table schema.
fn local_table_projection(
    projection: Option<&Vec<usize>>,
    federated_schema: &SchemaRef,
    local_schema: &SchemaRef,
) -> Option<Vec<usize>> {
    match projection {
        None => None,
        Some(idxs) => {
            let mapped: Vec<usize> = idxs
                .iter()
                .filter_map(|&i| {
                    let name = federated_schema.field(i).name();
                    local_schema.index_of(name).ok()
                })
                .collect();
            if mapped.is_empty() {
                None
            } else {
                Some(mapped)
            }
        }
    }
}

/// Federated mirror of a `probe` catalog table, exposed under the `global` catalog.
#[derive(Debug)]
pub struct GlobalFederatedTable {
    schema_name: String,
    table_name: String,
    local: Arc<dyn TableProvider>,
}

impl GlobalFederatedTable {
    pub fn new(
        schema_name: impl Into<String>,
        table_name: impl Into<String>,
        local: Arc<dyn TableProvider>,
    ) -> Self {
        Self {
            schema_name: schema_name.into(),
            table_name: table_name.into(),
            local,
        }
    }
}

#[async_trait]
impl TableProvider for GlobalFederatedTable {
    fn schema(&self) -> SchemaRef {
        federated_output_schema(self.local.schema())
    }

    fn table_type(&self) -> TableType {
        TableType::Base
    }

    async fn scan(
        &self,
        state: &dyn Session,
        projection: Option<&Vec<usize>>,
        filters: &[Expr],
        limit: Option<usize>,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        let output_schema = self.schema();
        let local_schema = self.local.schema();
        let local_projection = local_table_projection(projection, &output_schema, &local_schema);

        // Local scan stays lazy; coalesce to a single partition so the federated
        // plan can expose it as partition 0 without losing rows from sub-partitions.
        let local_plan = self
            .local
            .scan(state, local_projection.as_ref(), filters, limit)
            .await?;
        let local_plan: Arc<dyn ExecutionPlan> =
            Arc::new(CoalescePartitionsExec::new(local_plan));

        let host = ProbeClusterExecutor::local_host_label();
        let addr = ProbeClusterExecutor::local_addr_label();
        let local_rank = cluster_rank_for_endpoint(&host, &addr);

        reset_fanout_stats();
        let remote_sql = build_remote_table_sql(
            &self.schema_name,
            &self.table_name,
            &local_schema,
            local_projection.as_ref(),
            filters,
            limit,
        );
        let remote_nodes = ProbeClusterExecutor::remote_nodes();

        let scan_projection = federated_scan_projection(projection, &output_schema)
            .unwrap_or_else(|| (0..output_schema.fields().len()).collect());

        let exec = FederatedScanExec::try_new(
            local_plan,
            output_schema,
            scan_projection,
            remote_sql,
            remote_nodes,
            host,
            addr,
            local_rank,
        )?;
        Ok(Arc::new(exec))
    }

    fn supports_filters_pushdown(
        &self,
        filters: &[&Expr],
    ) -> Result<Vec<TableProviderFilterPushDown>> {
        self.local.supports_filters_pushdown(filters)
    }

    fn statistics(&self) -> Option<Statistics> {
        self.local.statistics()
    }
}
