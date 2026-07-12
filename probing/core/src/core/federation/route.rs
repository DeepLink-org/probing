//! Federated query routing classification and EXPLAIN helpers.
//!
//! Mirrors the path selection in `docs/src/design/federation.zh.md` §4.2:
//! - **AggregatePushdown** (A): single-table `global.*` + merge-safe aggregates
//! - **FederatedScan** (B): single-table `global.*` scan via `FederatedScanExec`
//! - **Broadcast** (C): JOIN / CTE / subquery — cluster fan-out only
//! - **Local**: `probe.*` or no federation catalog

use datafusion::arrow::record_batch::RecordBatch;
use datafusion::error::Result;

use crate::core::cluster::remote_peers_excluding_local;
use crate::core::Engine;

use super::aggregate_pushdown::{plan_federated_aggregate_pushdown, FederatedAggregatePlan};
use super::query_guard::global_scan_max_rows;
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

/// Runtime cluster shape used for federated path cost estimation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FederationRouteContext {
    /// Remote peers excluding the coordinator (matches fan-out target set).
    pub peer_count: usize,
}

impl FederationRouteContext {
    pub fn from_cluster() -> Self {
        Self {
            peer_count: remote_peers_excluding_local().len(),
        }
    }
}

/// Abstract cost model for federated execution paths (§4.2).
///
/// Units are relative — suitable for comparing paths and sizing fan-out concurrency,
/// not for wall-clock prediction without bandwidth/load telemetry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FederationCostEstimate {
    pub path: FederatedQueryPath,
    /// Coordinator + remote endpoints contacted (local + peers).
    pub peer_fanout: usize,
    /// Estimated rows returned per endpoint before merge.
    pub estimated_rows_per_peer: usize,
    /// Estimated rows materialized at the coordinator after merge.
    pub estimated_total_rows: usize,
    /// Relative cost units (higher = more network / merge work).
    pub relative_cost: u64,
    /// Suggested fan-out concurrency for this cluster size and path.
    pub recommended_fanout_concurrency: usize,
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
    pub route_context: FederationRouteContext,
    pub cost: FederationCostEstimate,
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

/// Classify a federated statement and attach a cluster-size-aware cost estimate.
pub fn classify_federated_sql_with_context(
    sql: &str,
    ctx: &FederationRouteContext,
) -> (FederatedQueryPath, FederationCostEstimate) {
    let path = classify_federated_sql(sql);
    let cost = estimate_federation_cost(path, ctx, sql);
    (path, cost)
}

/// Estimate relative cost for a federated path at the given cluster size.
pub fn estimate_federation_cost(
    path: FederatedQueryPath,
    ctx: &FederationRouteContext,
    sql: &str,
) -> FederationCostEstimate {
    let peer_fanout = match path {
        FederatedQueryPath::Local => 1,
        _ => ctx.peer_count.saturating_add(1),
    };
    let rows_per_peer = estimated_rows_per_peer(path, sql);
    let estimated_total_rows = match path {
        FederatedQueryPath::Local => rows_per_peer,
        FederatedQueryPath::AggregatePushdown => rows_per_peer.saturating_mul(peer_fanout),
        FederatedQueryPath::FederatedScan => rows_per_peer.saturating_mul(peer_fanout),
        FederatedQueryPath::Broadcast => rows_per_peer
            .saturating_mul(peer_fanout)
            .saturating_mul(peer_fanout),
    };
    let relative_cost = match path {
        FederatedQueryPath::Local => 1,
        FederatedQueryPath::AggregatePushdown => peer_fanout as u64 * 2,
        FederatedQueryPath::FederatedScan => {
            (peer_fanout as u64).saturating_mul(rows_per_peer as u64)
        }
        FederatedQueryPath::Broadcast => (peer_fanout as u64)
            .saturating_mul(peer_fanout as u64)
            .saturating_mul(rows_per_peer as u64),
    };
    FederationCostEstimate {
        path,
        peer_fanout,
        estimated_rows_per_peer: rows_per_peer,
        estimated_total_rows,
        relative_cost,
        recommended_fanout_concurrency: recommended_fanout_concurrency(ctx.peer_count),
    }
}

/// Fan-out concurrency tuned to cluster size (caps at [`super::cluster_executor::remote_fanout_concurrency`]).
pub fn recommended_fanout_concurrency(peer_count: usize) -> usize {
    let cap = super::cluster_executor::remote_fanout_concurrency();
    if peer_count == 0 {
        return cap;
    }
    peer_count.min(cap).max(1)
}

fn estimated_rows_per_peer(path: FederatedQueryPath, sql: &str) -> usize {
    match path {
        FederatedQueryPath::Local => sql_limit_value(sql).unwrap_or(1),
        FederatedQueryPath::AggregatePushdown => {
            // Partial aggregate groups are typically small vs raw scans.
            sql_limit_value(sql).unwrap_or(64)
        }
        FederatedQueryPath::FederatedScan => sql_limit_value(sql).unwrap_or(global_scan_max_rows()),
        FederatedQueryPath::Broadcast => sql_limit_value(sql).unwrap_or(1),
    }
}

fn sql_limit_value(sql: &str) -> Option<usize> {
    let upper = sql.to_ascii_uppercase();
    let idx = upper.rfind("LIMIT")?;
    let tail = sql[idx + 5..].trim();
    let num: String = tail.chars().take_while(|c| c.is_ascii_digit()).collect();
    num.parse().ok()
}

/// Build a full federation explain report: route + optional pushdown plan + physical EXPLAIN.
pub async fn explain_federation(
    engine: &Engine,
    user_sql: &str,
) -> Result<FederationExplainReport> {
    let global_sql = prepare_global_query(&rewrite_sql_for_global_fanout(user_sql));
    let route_context = FederationRouteContext::from_cluster();
    let (execution_path, cost) = classify_federated_sql_with_context(&global_sql, &route_context);
    let aggregate_plan = plan_federated_aggregate_pushdown(&global_sql);
    let physical_plan = explain_physical_plan(engine, &global_sql).await?;
    Ok(FederationExplainReport {
        user_sql: user_sql.to_string(),
        global_sql,
        execution_path,
        aggregate_plan,
        physical_plan,
        route_context,
        cost,
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

    #[test]
    fn cost_pushdown_cheaper_than_broadcast_at_scale() {
        let ctx = FederationRouteContext { peer_count: 128 };
        let pushdown_sql =
            "SELECT global_step, sum(duration_ms) FROM global.python.comm_collective GROUP BY global_step";
        let broadcast_sql =
            "SELECT a.x FROM global.python.a JOIN global.python.b ON a.id = b.id LIMIT 100";
        let (_, pushdown_cost) = classify_federated_sql_with_context(pushdown_sql, &ctx);
        let (_, broadcast_cost) = classify_federated_sql_with_context(broadcast_sql, &ctx);
        assert_eq!(pushdown_cost.path, FederatedQueryPath::AggregatePushdown);
        assert_eq!(broadcast_cost.path, FederatedQueryPath::Broadcast);
        assert!(pushdown_cost.relative_cost < broadcast_cost.relative_cost);
        assert_eq!(pushdown_cost.estimated_rows_per_peer, 64);
        assert_eq!(broadcast_cost.estimated_rows_per_peer, 100);
    }

    #[test]
    fn cost_scan_scales_with_peer_count_and_limit() {
        let ctx = FederationRouteContext { peer_count: 10 };
        let sql = "SELECT rank FROM global.demo.metrics WHERE rank > 0 LIMIT 50";
        let (_, cost) = classify_federated_sql_with_context(sql, &ctx);
        assert_eq!(cost.path, FederatedQueryPath::FederatedScan);
        assert_eq!(cost.peer_fanout, 11);
        assert_eq!(cost.estimated_rows_per_peer, 50);
        assert_eq!(cost.estimated_total_rows, 550);
    }

    #[test]
    fn recommended_fanout_scales_down_for_small_clusters() {
        assert_eq!(recommended_fanout_concurrency(4), 4);
        assert!(recommended_fanout_concurrency(512) <= 128);
    }
}
