//! Integration tests: federated routing + EXPLAIN plan shape (§4.2 path A/B/C).

use std::sync::Arc;

use probing_core::core::cluster::{reset_cluster_for_tests, update_node};
use probing_core::core::federation::{
    classify_cluster_sql, classify_federated_sql, explain_federation, explain_physical_plan,
    plan_federated_aggregate_pushdown, prepare_global_query, FederatedQueryPath,
};
use probing_core::core::{Engine, ProbeDataSource};
use probing_proto::prelude::Node;
use probing_rust_regression::test_helpers::{federation_test_lock, GenericTableProbeDataSource};

async fn metrics_engine(values: Vec<i32>) -> Engine {
    std::env::set_var("PROBING_ADDRESS", "127.0.0.1:19999");
    std::env::set_var("HOSTNAME", "explain-coord");
    reset_cluster_for_tests();
    update_node(Node {
        host: "explain-coord".into(),
        addr: "127.0.0.1:19999".into(),
        rank: Some(0),
        ..Default::default()
    });
    update_node(Node {
        host: "explain-peer".into(),
        addr: "127.0.0.1:20001".into(),
        rank: Some(1),
        ..Default::default()
    });

    let table = GenericTableProbeDataSource::single_column_table("metrics", "demo", "v", values);
    Engine::builder()
        .with_data_source(Arc::new(table) as Arc<dyn ProbeDataSource + Send + Sync>)
        .build()
        .await
        .expect("engine")
}

fn assert_plan_contains(plan: &str, needle: &str, context: &str) {
    assert!(
        plan.contains(needle),
        "{context}: expected EXPLAIN to contain `{needle}`\n--- plan ---\n{plan}"
    );
}

// --- Route classification (design §4.2 matrix) ---

#[test]
fn route_matrix_aggregate_pushdown_comm_heatmap() {
    let sql = "SELECT global_step, _rank, sum(duration_ms) AS comm_ms \
               FROM python.comm_collective \
               WHERE global_step >= 0 \
               GROUP BY global_step, _rank";
    assert_eq!(
        classify_cluster_sql(sql),
        FederatedQueryPath::AggregatePushdown
    );
}

#[test]
fn route_matrix_federated_scan_raw_rows() {
    let sql = "SELECT rank FROM python.comm_collective WHERE rank > 0 LIMIT 100";
    assert_eq!(classify_cluster_sql(sql), FederatedQueryPath::FederatedScan);
}

#[test]
fn route_matrix_broadcast_join_compute_vs_comm() {
    let sql = "SELECT c.global_step, sum(c.duration_ms) AS comm_ms \
               FROM python.comm_collective c \
               JOIN python.torch_trace t \
                 ON c.global_step = t.global_step AND c.rank = t.rank \
               GROUP BY c.global_step";
    assert_eq!(classify_cluster_sql(sql), FederatedQueryPath::Broadcast);
}

#[test]
fn route_matrix_broadcast_cte_slowdown() {
    let sql = "WITH per_rank AS ( \
                 SELECT global_step, _rank, max(duration_ms) AS max_ms \
                 FROM python.comm_collective GROUP BY global_step, _rank \
               ) SELECT avg(max_ms) FROM per_rank";
    assert_eq!(classify_cluster_sql(sql), FederatedQueryPath::Broadcast);
}

#[test]
fn route_matrix_local_probe_catalog() {
    assert_eq!(
        classify_federated_sql("SELECT v FROM probe.demo.metrics"),
        FederatedQueryPath::Local
    );
}

// --- EXPLAIN physical plan shape (path B) ---

#[tokio::test]
async fn explain_federated_scan_exec_for_single_table_scan() {
    let _lock = federation_test_lock().await;
    let engine = metrics_engine(vec![1, 2, 3]).await;

    let sql = "SELECT v FROM global.demo.metrics WHERE v > 0";
    let plan = explain_physical_plan(&engine, &prepare_global_query(sql))
        .await
        .expect("explain");

    assert_plan_contains(&plan, "FederatedScanExec", "path B single-table scan");
    assert_plan_contains(&plan, "global.demo.metrics", "logical table scan");
    assert_plan_contains(
        &plan,
        "remote_sql=SELECT \"v\" FROM probe.demo.metrics",
        "peer SQL uses probe catalog",
    );
}

#[tokio::test]
async fn explain_federated_scan_with_peers_shows_peer_count() {
    let _lock = federation_test_lock().await;
    let engine = metrics_engine(vec![1, 2]).await;

    let sql = "SELECT v FROM global.demo.metrics";
    let plan = explain_physical_plan(&engine, &prepare_global_query(sql))
        .await
        .expect("explain");

    // One registered peer → FederatedScanExec: peers=1
    assert_plan_contains(&plan, "FederatedScanExec: peers=1", "peer partition count");
}

#[tokio::test]
async fn explain_aggregate_query_still_plans_federated_scan_underneath() {
    let _lock = federation_test_lock().await;
    let engine = metrics_engine(vec![1, 2, 3]).await;

    let sql = "SELECT sum(v) AS total FROM global.demo.metrics";
    let plan = explain_physical_plan(&engine, sql).await.expect("explain");

    // DataFusion EXPLAIN: scan then aggregate locally on coordinator.
    assert_plan_contains(&plan, "FederatedScanExec", "scan under aggregate");
    assert_plan_contains(&plan, "AggregateExec", "partial/global aggregate in plan");
}

// --- Path A pushdown plan contract (execution differs from EXPLAIN) ---

#[test]
fn pushdown_plan_per_node_uses_probe_and_strips_tags() {
    let global_sql = prepare_global_query(
        "SELECT _host, sum(v) AS total FROM global.demo.metrics GROUP BY _host ORDER BY total DESC LIMIT 5",
    );
    let plan = plan_federated_aggregate_pushdown(&global_sql).expect("pushdown plan");

    assert!(plan.per_node_sql.contains("probe.demo.metrics"));
    assert!(!plan.per_node_sql.contains("global."));
    assert!(!plan.per_node_sql.to_uppercase().contains("_HOST"));
    assert!(!plan.per_node_sql.to_uppercase().contains("ORDER BY"));
    assert!(!plan.per_node_sql.to_uppercase().contains("LIMIT"));
    let tail = plan.post_merge_tail.as_deref().unwrap_or("");
    assert!(tail.contains("ORDER BY total DESC"));
    assert!(tail.contains("LIMIT 5"));
}

#[tokio::test]
async fn explain_federation_report_matches_design_paths() {
    let _lock = federation_test_lock().await;
    let engine = metrics_engine(vec![10, 20]).await;

    // Path A: execution = pushdown; report carries plan + EXPLAIN scan/agg shape.
    let heatmap = explain_federation(
        &engine,
        "SELECT v, sum(v) AS s FROM global.demo.metrics GROUP BY v ORDER BY s DESC LIMIT 3",
    )
    .await
    .expect("explain federation");
    assert_eq!(
        heatmap.execution_path,
        FederatedQueryPath::AggregatePushdown
    );
    assert!(heatmap.aggregate_plan.is_some());
    assert!(heatmap.physical_plan.contains("FederatedScanExec"));
    assert!(heatmap.global_sql.contains("global.demo.metrics"));

    // Path B: raw scan.
    let scan = explain_federation(&engine, "SELECT v FROM global.demo.metrics WHERE v > 5")
        .await
        .expect("scan explain");
    assert_eq!(scan.execution_path, FederatedQueryPath::FederatedScan);
    assert!(scan.aggregate_plan.is_none());
    assert!(scan.physical_plan.contains("FederatedScanExec"));

    // Path C: join.
    let join = explain_federation(
        &engine,
        "SELECT a.v FROM global.demo.metrics a JOIN global.demo.metrics b ON a.v = b.v",
    )
    .await
    .expect("join explain");
    assert_eq!(join.execution_path, FederatedQueryPath::Broadcast);
}

#[tokio::test]
async fn explain_select_star_rewrite_before_plan() {
    let _lock = federation_test_lock().await;
    let engine = metrics_engine(vec![1]).await;

    let report = explain_federation(&engine, "SELECT * FROM global.demo.metrics")
        .await
        .expect("report");

    assert_eq!(report.global_sql, "SELECT * FROM global.demo.metrics");
    assert_eq!(report.execution_path, FederatedQueryPath::FederatedScan);
}
