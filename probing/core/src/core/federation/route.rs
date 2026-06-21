//! Federated query routing classification and EXPLAIN helpers.
//!
//! Mirrors the path selection in `docs/src/design/federation.zh.md` §4.2:
//! - **AggregatePushdown** (A): single-table `global.*` + merge-safe aggregates
//! - **FederatedScan** (B): single-table `global.*` scan via `FederatedScanExec`
//! - **Broadcast** (C): JOIN / CTE / subquery — cluster fan-out only
//! - **Local**: `probe.*` or no federation catalog

use datafusion::arrow::record_batch::RecordBatch;
use datafusion::error::Result;

use crate::core::Engine;

use super::aggregate_pushdown::{plan_federated_aggregate_pushdown, FederatedAggregatePlan};
use super::rewrite::{
    can_fanout_via_global_catalog, prepare_global_query, rewrite_sql_for_global_fanout,
};

/// Execution path for a federated SQL statement (coordinator view).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FederatedQueryPath {
    /// Single-process `probe.*` (no `global.*` / known schema fan-out).
    Local,
    /// Path A — partial aggregates on each peer, merge on coordinator.
    AggregatePushdown,
    /// Path B — lazy `FederatedScanExec` over local + peers.
    FederatedScan,
    /// Path C — broadcast full SQL to each rank (JOIN / CTE / …).
    Broadcast,
}

/// Snapshot returned by [`explain_federation`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FederationExplainReport {
    pub user_sql: String,
    pub global_sql: String,
    pub execution_path: FederatedQueryPath,
    pub aggregate_plan: Option<FederatedAggregatePlan>,
    /// DataFusion `EXPLAIN` text for the prepared `global.*` statement (path B plan shape).
    pub physical_plan: String,
}

/// Classify a SQL string that already references `global.*` (or will after rewrite).
pub fn classify_federated_sql(sql: &str) -> FederatedQueryPath {
    let lower = sql.to_lowercase();
    if !lower.contains("global.") {
        return FederatedQueryPath::Local;
    }
    if !can_fanout_via_global_catalog(sql) {
        return FederatedQueryPath::Broadcast;
    }
    if plan_federated_aggregate_pushdown(sql).is_some() {
        return FederatedQueryPath::AggregatePushdown;
    }
    FederatedQueryPath::FederatedScan
}

/// Classify user/cluster SQL (`python.t` → `global.*` rewrite applied first).
pub fn classify_cluster_sql(user_sql: &str) -> FederatedQueryPath {
    classify_federated_sql(&rewrite_sql_for_global_fanout(user_sql))
}

/// Build a full federation explain report: route + optional pushdown plan + physical EXPLAIN.
pub async fn explain_federation(
    engine: &Engine,
    user_sql: &str,
) -> Result<FederationExplainReport> {
    let global_sql = prepare_global_query(&rewrite_sql_for_global_fanout(user_sql));
    let execution_path = classify_federated_sql(&global_sql);
    let aggregate_plan = plan_federated_aggregate_pushdown(&global_sql);
    let physical_plan = explain_physical_plan(engine, &global_sql).await?;
    Ok(FederationExplainReport {
        user_sql: user_sql.to_string(),
        global_sql,
        execution_path,
        aggregate_plan,
        physical_plan,
    })
}

/// Run `EXPLAIN` on a prepared SQL string and return the plan text.
pub async fn explain_physical_plan(engine: &Engine, sql: &str) -> Result<String> {
    let df = engine.context.sql(&format!("EXPLAIN {sql}")).await?;
    let batches = df.collect().await?;
    Ok(format_explain_batches(&batches))
}

fn format_explain_batches(batches: &[RecordBatch]) -> String {
    let mut lines = Vec::new();
    for batch in batches {
        let schema = batch.schema();
        for row in 0..batch.num_rows() {
            let mut parts = Vec::new();
            for col in 0..batch.num_columns() {
                let name = schema.field(col).name();
                let array = batch.column(col);
                let value = arrow::util::display::array_value_to_string(array, row)
                    .unwrap_or_else(|_| "?".to_string());
                if parts.is_empty() && schema.fields().len() == 1 {
                    lines.push(value);
                } else {
                    parts.push(format!("{name}={value}"));
                }
            }
            if !parts.is_empty() {
                lines.push(parts.join(" "));
            }
        }
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_aggregate_pushdown() {
        let sql = "SELECT global_step, sum(duration_ms) AS ms \
                   FROM global.python.comm_collective GROUP BY global_step";
        assert_eq!(
            classify_federated_sql(sql),
            FederatedQueryPath::AggregatePushdown
        );
    }

    #[test]
    fn classify_federated_scan() {
        let sql = "SELECT rank FROM global.demo.metrics WHERE rank > 0";
        assert_eq!(
            classify_federated_sql(sql),
            FederatedQueryPath::FederatedScan
        );
    }

    #[test]
    fn classify_broadcast_join() {
        let sql = "SELECT a.x FROM global.python.a JOIN global.python.b ON a.id = b.id";
        assert_eq!(classify_federated_sql(sql), FederatedQueryPath::Broadcast);
    }

    #[test]
    fn classify_cluster_rewrite_to_global() {
        let sql = "SELECT rank, sum(duration_ms) FROM python.comm_collective GROUP BY rank";
        assert_eq!(
            classify_cluster_sql(sql),
            FederatedQueryPath::AggregatePushdown
        );
    }
}
