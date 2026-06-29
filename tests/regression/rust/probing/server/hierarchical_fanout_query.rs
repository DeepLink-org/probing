//! Hierarchical cluster query fan-out integration test (mock HTTP peers + real engine).

use std::sync::{LazyLock, Mutex};

use axum::{extract::State, http::StatusCode, routing::post, Json, Router};
use probing_core::core::cluster::{reset_cluster_for_tests, set_local_listen_addrs, update_node};
use probing_proto::prelude::{DataFrame, Message, Node, QueryDataFormat, Seq};
use probing_server::initialize_engine;
use probing_server::server::cluster_fanout::{
    fanout_query, ClusterFanoutScope, FanoutMeta, FanoutQueryResponse,
};
use probing_server::server::cluster_query::ClusterQueryRequest;
use probing_server::server::SERVER_RUNTIME;
use tokio::net::TcpListener;

static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

#[derive(Clone)]
struct QueryState {
    rank: i32,
}

#[derive(Clone)]
struct NodeAggState {
    meta: FanoutMeta,
    rank: i32,
}

fn test_node(rank: i32, group_rank: i32, local_rank: i32, addr: &str) -> Node {
    Node {
        host: "127.0.0.1".into(),
        addr: addr.into(),
        rank: Some(rank),
        group_rank: Some(group_rank),
        local_rank: Some(local_rank),
        world_size: Some(4),
        group_world_size: Some(2),
        status: Some("running".into()),
        ..Default::default()
    }
}

fn rank_dataframe(rank: i32) -> DataFrame {
    DataFrame {
        names: vec!["rank".into()],
        cols: vec![Seq::SeqI32(vec![rank])],
        size: 1,
    }
}

fn query_message(rank: i32) -> String {
    serde_json::to_string(&Message::new(QueryDataFormat::DataFrame(rank_dataframe(
        rank,
    ))))
    .expect("serialize query response")
}

async fn query_handler(State(state): State<QueryState>, body: String) -> (StatusCode, String) {
    assert!(
        body.contains("SELECT"),
        "expected SQL query body, got: {body}"
    );
    (StatusCode::OK, query_message(state.rank))
}

async fn node_aggregate_handler(
    State(state): State<NodeAggState>,
    Json(body): Json<ClusterQueryRequest>,
) -> Json<FanoutQueryResponse> {
    assert!(body.cluster, "node aggregate expects cluster=true");
    assert!(body.hierarchical);
    assert_eq!(body.scope, ClusterFanoutScope::Node);
    Json(FanoutQueryResponse {
        dataframe: rank_dataframe(state.rank),
        meta: state.meta.clone(),
    })
}

async fn spawn_query_server(rank: i32) -> String {
    let app = Router::new()
        .route("/query", post(query_handler))
        .with_state(QueryState { rank });
    bind_http(app).await
}

async fn spawn_node_aggregate_server(rank: i32, meta: FanoutMeta) -> String {
    let app = Router::new()
        .route("/apis/cluster/query", post(node_aggregate_handler))
        .with_state(NodeAggState { rank, meta });
    bind_http(app).await
}

async fn bind_http(app: Router) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("{addr}")
}

fn install_cluster(
    coordinator_addr: &str,
    leaf_addr: &str,
    node1_local0_addr: &str,
    node1_leaf_addr: &str,
) {
    reset_cluster_for_tests();
    set_local_listen_addrs(vec![coordinator_addr.into()]);
    for node in [
        test_node(0, 0, 0, coordinator_addr),
        test_node(1, 0, 1, leaf_addr),
        test_node(2, 1, 0, node1_local0_addr),
        test_node(3, 1, 1, node1_leaf_addr),
    ] {
        update_node(node);
    }
}

fn install_cluster_without_hierarchical_metadata(coordinator_addr: &str, peer_addrs: &[&str]) {
    reset_cluster_for_tests();
    set_local_listen_addrs(vec![coordinator_addr.into()]);
    for (rank, addr) in peer_addrs.iter().enumerate() {
        update_node(Node {
            host: "127.0.0.1".into(),
            addr: (*addr).into(),
            rank: Some(rank as i32),
            world_size: Some(peer_addrs.len() as i32),
            status: Some("running".into()),
            ..Default::default()
        });
    }
}

async fn run_hierarchical_fanout() -> FanoutQueryResponse {
    fanout_query("SELECT 1 AS n", true, true, ClusterFanoutScope::Auto)
        .await
        .expect("hierarchical fanout")
}

fn set_coordinator_env() {
    std::env::set_var("RANK", "0");
    std::env::set_var("LOCAL_RANK", "0");
    std::env::set_var("GROUP_RANK", "0");
    std::env::set_var("PROBING_CLUSTER_FANOUT_HIERARCHICAL", "1");
}

fn clear_rank_env() {
    for key in [
        "RANK",
        "LOCAL_RANK",
        "GROUP_RANK",
        "PROBING_CLUSTER_FANOUT_HIERARCHICAL",
    ] {
        std::env::remove_var(key);
    }
}

#[test]
fn hierarchical_fanout_contacts_node_aggregators_not_every_rank() {
    let _guard = ENV_LOCK.lock().unwrap();
    clear_rank_env();

    SERVER_RUNTIME.block_on(async {
        initialize_engine()
            .await
            .expect("initialize probing engine");

        let leaf_addr = spawn_query_server(1).await;
        let node1_local0 = spawn_node_aggregate_server(
            2,
            FanoutMeta {
                cluster: true,
                hierarchical: true,
                scope: "node".into(),
                nodes_queried: 2,
                nodes_failed: Vec::new(),
                node_aggregators_queried: 0,
                local_ranks_queried: 1,
            },
        )
        .await;
        let node1_leaf = spawn_query_server(3).await;

        let coordinator_addr = "127.0.0.1:59999";
        install_cluster(coordinator_addr, &leaf_addr, &node1_local0, &node1_leaf);
        set_coordinator_env();

        let result = fanout_query("SELECT 1 AS n", true, true, ClusterFanoutScope::Auto)
            .await
            .expect("hierarchical fanout");

        assert!(result.meta.hierarchical);
        assert_eq!(result.meta.scope, "coordinator");
        assert_eq!(result.meta.node_aggregators_queried, 1);
        assert_eq!(result.meta.local_ranks_queried, 1);
        assert_eq!(result.meta.nodes_queried, 3);
        assert!(
            result.meta.nodes_failed.is_empty(),
            "unexpected failures: {:?}",
            result.meta.nodes_failed
        );
    });

    clear_rank_env();
}

#[test]
fn flat_fanout_contacts_all_remote_peers() {
    let _guard = ENV_LOCK.lock().unwrap();
    clear_rank_env();

    SERVER_RUNTIME.block_on(async {
        initialize_engine()
            .await
            .expect("initialize probing engine");

        let leaf_addr = spawn_query_server(1).await;
        let node1_local0 = spawn_query_server(2).await;
        let node1_leaf = spawn_query_server(3).await;

        let coordinator_addr = "127.0.0.1:59998";
        install_cluster(coordinator_addr, &leaf_addr, &node1_local0, &node1_leaf);
        set_coordinator_env();
        std::env::set_var("PROBING_CLUSTER_FANOUT_HIERARCHICAL", "0");

        let result = fanout_query("SELECT 1 AS n", true, false, ClusterFanoutScope::Auto)
            .await
            .expect("flat fanout");

        assert!(!result.meta.hierarchical);
        assert_eq!(result.meta.scope, "flat");
        assert_eq!(result.meta.node_aggregators_queried, 0);
        assert_eq!(result.meta.local_ranks_queried, 0);
        // local coordinator + 3 remote peers
        assert_eq!(result.meta.nodes_queried, 4);
        assert!(result.meta.nodes_failed.is_empty());
    });

    clear_rank_env();
}

#[test]
fn hierarchical_fanout_degrades_to_flat_without_metadata() {
    let _guard = ENV_LOCK.lock().unwrap();
    clear_rank_env();

    SERVER_RUNTIME.block_on(async {
        initialize_engine()
            .await
            .expect("initialize probing engine");

        let peer1 = spawn_query_server(1).await;
        let peer2 = spawn_query_server(2).await;
        let peer3 = spawn_query_server(3).await;

        let coordinator_addr = "127.0.0.1:59997";
        install_cluster_without_hierarchical_metadata(
            coordinator_addr,
            &[coordinator_addr, &peer1, &peer2, &peer3],
        );
        set_coordinator_env();

        let result = run_hierarchical_fanout().await;

        assert!(
            !result.meta.hierarchical,
            "missing group_rank/local_rank should fall back to flat fan-out"
        );
        assert_eq!(result.meta.scope, "flat");
        assert_eq!(result.meta.node_aggregators_queried, 0);
        assert_eq!(result.meta.local_ranks_queried, 0);
        assert_eq!(result.meta.nodes_queried, 4);
        assert!(result.meta.nodes_failed.is_empty());
    });

    clear_rank_env();
}

#[test]
fn hierarchical_fanout_reports_failed_remote_node_aggregator() {
    let _guard = ENV_LOCK.lock().unwrap();
    clear_rank_env();

    SERVER_RUNTIME.block_on(async {
        initialize_engine()
            .await
            .expect("initialize probing engine");

        let leaf_addr = spawn_query_server(1).await;
        let dead_node_agg = "127.0.0.1:1";
        let node1_leaf = spawn_query_server(3).await;

        let coordinator_addr = "127.0.0.1:59996";
        install_cluster(coordinator_addr, &leaf_addr, dead_node_agg, &node1_leaf);
        set_coordinator_env();

        let result = run_hierarchical_fanout().await;

        assert!(result.meta.hierarchical);
        assert_eq!(result.meta.scope, "coordinator");
        assert_eq!(result.meta.node_aggregators_queried, 1);
        assert_eq!(result.meta.local_ranks_queried, 1);
        assert_eq!(result.meta.nodes_queried, 2, "local0 + local leaf only");
        assert_eq!(result.meta.nodes_failed, vec![dead_node_agg.to_string()]);
    });

    clear_rank_env();
}

#[test]
fn hierarchical_fanout_reports_failed_local_leaf() {
    let _guard = ENV_LOCK.lock().unwrap();
    clear_rank_env();

    SERVER_RUNTIME.block_on(async {
        initialize_engine()
            .await
            .expect("initialize probing engine");

        let dead_leaf = "127.0.0.1:2";
        let node1_local0 = spawn_node_aggregate_server(
            2,
            FanoutMeta {
                cluster: true,
                hierarchical: true,
                scope: "node".into(),
                nodes_queried: 2,
                nodes_failed: Vec::new(),
                node_aggregators_queried: 0,
                local_ranks_queried: 1,
            },
        )
        .await;
        let node1_leaf = spawn_query_server(3).await;

        let coordinator_addr = "127.0.0.1:59995";
        install_cluster(coordinator_addr, dead_leaf, &node1_local0, &node1_leaf);
        set_coordinator_env();

        let result = run_hierarchical_fanout().await;

        assert!(result.meta.hierarchical);
        assert_eq!(result.meta.scope, "coordinator");
        assert_eq!(result.meta.node_aggregators_queried, 1);
        assert_eq!(result.meta.local_ranks_queried, 1);
        assert_eq!(result.meta.nodes_queried, 2, "local0 + remote node agg");
        assert_eq!(result.meta.nodes_failed, vec![dead_leaf.to_string()]);
    });

    clear_rank_env();
}

#[test]
fn hierarchical_fanout_leaf_rank_stays_local_only() {
    let _guard = ENV_LOCK.lock().unwrap();
    clear_rank_env();

    SERVER_RUNTIME.block_on(async {
        initialize_engine()
            .await
            .expect("initialize probing engine");

        let leaf_addr = spawn_query_server(1).await;
        let node1_local0 = spawn_query_server(2).await;
        let node1_leaf = spawn_query_server(3).await;

        let coordinator_addr = "127.0.0.1:59994";
        install_cluster(coordinator_addr, &leaf_addr, &node1_local0, &node1_leaf);
        std::env::set_var("RANK", "1");
        std::env::set_var("LOCAL_RANK", "1");
        std::env::set_var("GROUP_RANK", "0");
        std::env::set_var("PROBING_CLUSTER_FANOUT_HIERARCHICAL", "1");
        set_local_listen_addrs(vec![leaf_addr.clone()]);

        let result = fanout_query("SELECT 1 AS n", true, true, ClusterFanoutScope::Auto)
            .await
            .expect("leaf rank fanout");

        assert!(result.meta.hierarchical);
        assert_eq!(result.meta.scope, "local");
        assert_eq!(result.meta.nodes_queried, 1);
        assert_eq!(result.meta.node_aggregators_queried, 0);
        assert_eq!(result.meta.local_ranks_queried, 0);
        assert!(result.meta.nodes_failed.is_empty());
    });

    clear_rank_env();
}
