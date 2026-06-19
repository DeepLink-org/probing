//! Streaming, lazy execution plan for `global.*` federated table scans.
//!
//! Instead of eagerly collecting the local table and every peer node into one
//! in-memory buffer (which makes the coordinator's peak memory scale with the
//! whole cluster's result), this plan exposes one partition per data source:
//!
//! * partition `0` streams the local `probe` table (tagged with node columns),
//! * partitions `1..=N` lazily fetch one peer each when first polled.
//!
//! Downstream operators (e.g. `LIMIT`) can therefore consume incrementally and
//! short-circuit without forcing every peer's rows to be materialized at once.

use std::fmt;
use std::sync::Arc;

use datafusion::arrow::datatypes::SchemaRef;
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::error::{DataFusionError, Result};
use datafusion::execution::TaskContext;
use datafusion::physical_expr::EquivalenceProperties;
use datafusion::physical_plan::execution_plan::{Boundedness, EmissionType};
use datafusion::physical_plan::stream::RecordBatchStreamAdapter;
use datafusion::physical_plan::{
    DisplayAs, DisplayFormatType, ExecutionPlan, Partitioning, PlanProperties,
    SendableRecordBatchStream,
};
use futures::StreamExt;
use probing_proto::prelude::{DataFrame, Node};

use super::cluster_executor::{
    record_fanout_failure, record_fanout_success, ProbeClusterExecutor,
};
use super::convert::{align_batch_to_schema, dataframe_to_record_batch, tag_record_batch};

/// Federated scan plan combining the local table with lazily fetched peers.
#[derive(Debug)]
pub struct FederatedScanExec {
    /// Local `probe` scan, coalesced to a single partition by the caller.
    local: Arc<dyn ExecutionPlan>,
    /// Full federated output schema (local columns + `_host`/`_addr`/`_rank`).
    output_schema: SchemaRef,
    /// Schema after applying [`projection`](Self::projection).
    projected_schema: SchemaRef,
    /// Column indices (into `output_schema`) to emit, honoring the scan projection.
    projection: Vec<usize>,
    /// `probe.*` SQL executed on each peer node.
    remote_sql: String,
    /// Snapshot of peer nodes captured at planning time (one partition each).
    remote_nodes: Vec<Node>,
    local_host: String,
    local_addr: String,
    local_rank: Option<i32>,
    properties: Arc<PlanProperties>,
}

impl FederatedScanExec {
    pub fn try_new(
        local: Arc<dyn ExecutionPlan>,
        output_schema: SchemaRef,
        projection: Vec<usize>,
        remote_sql: String,
        remote_nodes: Vec<Node>,
        local_host: String,
        local_addr: String,
        local_rank: Option<i32>,
    ) -> Result<Self> {
        let projected_schema = Arc::new(
            output_schema
                .project(&projection)
                .map_err(DataFusionError::from)?,
        );
        let num_partitions = 1 + remote_nodes.len();
        let properties = PlanProperties::new(
            EquivalenceProperties::new(projected_schema.clone()),
            Partitioning::UnknownPartitioning(num_partitions),
            EmissionType::Incremental,
            Boundedness::Bounded,
        );
        Ok(Self {
            local,
            output_schema,
            projected_schema,
            projection,
            remote_sql,
            remote_nodes,
            local_host,
            local_addr,
            local_rank,
            properties: Arc::new(properties),
        })
    }

    fn execute_local(&self, context: Arc<TaskContext>) -> Result<SendableRecordBatchStream> {
        let input = self.local.execute(0, context)?;
        let host = self.local_host.clone();
        let addr = self.local_addr.clone();
        let rank = self.local_rank;
        let full = self.output_schema.clone();
        let projection = self.projection.clone();
        let projected_schema = self.projected_schema.clone();
        let mapped = input.map(move |batch| {
            let batch = batch?;
            let tagged = tag_record_batch(batch, &host, &addr, rank)?;
            finalize_aligned_batch(tagged, full.as_ref(), &projection)
        });
        Ok(Box::pin(RecordBatchStreamAdapter::new(
            projected_schema,
            mapped,
        )))
    }

    fn execute_remote(
        &self,
        node_index: usize,
    ) -> Result<SendableRecordBatchStream> {
        let node = self.remote_nodes.get(node_index).ok_or_else(|| {
            DataFusionError::Internal(format!(
                "FederatedScanExec: no peer node at index {node_index}"
            ))
        })?;
        let addr_query = node.addr.clone();
        let addr_tag = node.addr.clone();
        let host = if node.host.is_empty() {
            node.addr.clone()
        } else {
            node.host.clone()
        };
        let rank = node.rank;
        let sql = self.remote_sql.clone();
        let full = self.output_schema.clone();
        let projection = self.projection.clone();
        let projected_schema = self.projected_schema.clone();

        // Best-effort fetch: failures (network, conversion) drop the node from
        // the result set and are recorded in the fan-out stats rather than
        // failing the whole query, matching the legacy partial-result behavior.
        let fut = async move {
            let joined = tokio::task::spawn_blocking(move || {
                ProbeClusterExecutor::execute_remote_query(&addr_query, &sql)
            })
            .await;
            match joined {
                Ok(Ok(df)) => match finalize_remote_dataframe(
                    &df,
                    &host,
                    &addr_tag,
                    rank,
                    full.as_ref(),
                    &projection,
                ) {
                    Ok(opt) => {
                        record_fanout_success();
                        opt
                    }
                    Err(err) => {
                        log::debug!("federated scan dropped {addr_tag}: {err}");
                        record_fanout_failure(&addr_tag);
                        None
                    }
                },
                Ok(Err(err)) => {
                    log::debug!("federated scan skipped {addr_tag}: {err}");
                    record_fanout_failure(&addr_tag);
                    None
                }
                Err(err) => {
                    log::debug!("federated scan join failed for {addr_tag}: {err}");
                    record_fanout_failure(&addr_tag);
                    None
                }
            }
        };
        let stream = futures::stream::once(fut)
            .filter_map(|opt| futures::future::ready(opt.map(Ok::<_, DataFusionError>)));
        Ok(Box::pin(RecordBatchStreamAdapter::new(
            projected_schema,
            stream,
        )))
    }
}

/// Align an already-tagged batch to the full federated schema, then project.
fn finalize_aligned_batch(
    batch: RecordBatch,
    full_schema: &datafusion::arrow::datatypes::Schema,
    projection: &[usize],
) -> Result<RecordBatch> {
    let aligned = align_batch_to_schema(batch, full_schema)?;
    aligned.project(projection).map_err(DataFusionError::from)
}

/// Convert a peer's dataframe to a projected, schema-aligned batch.
///
/// Returns `None` for empty results so empty peers contribute nothing to the stream.
fn finalize_remote_dataframe(
    df: &DataFrame,
    host: &str,
    addr: &str,
    rank: Option<i32>,
    full_schema: &datafusion::arrow::datatypes::Schema,
    projection: &[usize],
) -> Result<Option<RecordBatch>> {
    if df.is_empty() {
        return Ok(None);
    }
    let batch = dataframe_to_record_batch(df, host, addr, rank)?;
    if batch.num_rows() == 0 {
        return Ok(None);
    }
    Ok(Some(finalize_aligned_batch(batch, full_schema, projection)?))
}

impl DisplayAs for FederatedScanExec {
    fn fmt_as(&self, _t: DisplayFormatType, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "FederatedScanExec: peers={}, remote_sql={}",
            self.remote_nodes.len(),
            self.remote_sql
        )
    }
}

impl ExecutionPlan for FederatedScanExec {
    fn name(&self) -> &str {
        "FederatedScanExec"
    }

    fn properties(&self) -> &Arc<PlanProperties> {
        &self.properties
    }

    fn children(&self) -> Vec<&Arc<dyn ExecutionPlan>> {
        vec![&self.local]
    }

    fn with_new_children(
        self: Arc<Self>,
        children: Vec<Arc<dyn ExecutionPlan>>,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        let local = children.into_iter().next().ok_or_else(|| {
            DataFusionError::Internal("FederatedScanExec expects exactly one child".into())
        })?;
        Ok(Arc::new(FederatedScanExec {
            local,
            output_schema: self.output_schema.clone(),
            projected_schema: self.projected_schema.clone(),
            projection: self.projection.clone(),
            remote_sql: self.remote_sql.clone(),
            remote_nodes: self.remote_nodes.clone(),
            local_host: self.local_host.clone(),
            local_addr: self.local_addr.clone(),
            local_rank: self.local_rank,
            properties: self.properties.clone(),
        }))
    }

    fn execute(
        &self,
        partition: usize,
        context: Arc<TaskContext>,
    ) -> Result<SendableRecordBatchStream> {
        if partition == 0 {
            self.execute_local(context)
        } else {
            self.execute_remote(partition - 1)
        }
    }
}
