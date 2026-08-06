//! Hierarchical cluster report integration test (mock HTTP servers + PUT merge path).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::{extract::State, middleware, routing::get, routing::post, Json, Router};
use probing_core::core::federation::{FanoutScope, ProbeClusterExecutor};
use probing_core::sync::lock_mutex;
use probing_proto::prelude::{
    Cluster, DataFrame, Message, Node, NodeListResponse, NodeReportRequest, NodeReportResponse,
    QueryDataFormat,
};
use probing_server::auth::{
    bootstrap_auth_from_env, persist_auth_token, selective_auth_middleware, AUTH_TOKEN_ENV,
};
use probing_server::cluster_http::{fetch_nodes_blocking, put_nodes_blocking};
use probing_server::server::cluster_fanout::remote_query_df;
use probing_server::server::SERVER_RUNTIME;
use tokio::net::TcpListener;

#[derive(Clone)]
struct AppState {
    cluster: Arc<Mutex<Cluster>>,
    version: Arc<AtomicU64>,
}

static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

fn local_http_available() -> bool {
    match SERVER_RUNTIME.try_block_on(TcpListener::bind("127.0.0.1:0")) {
        Ok(Ok(listener)) => {
            drop(listener);
            true
        }
        Ok(Err(error)) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            eprintln!(
                "skipping hierarchical cluster report test: environment denied TCP bind ({error})"
            );
            false
        }
        Ok(Err(error)) => panic!("probe local HTTP bind capability: {error}"),
        Err(error) => panic!("probe runtime unavailable: {error}"),
    }
}

fn now_micros() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_micros() as u64
}

fn test_node(rank: i32, group_rank: i32, addr: &str) -> Node {
    Node {
        host: "127.0.0.1".into(),
        addr: addr.into(),
        rank: Some(rank),
        group_rank: Some(group_rank),
        world_size: Some(4),
        group_world_size: Some(2),
        local_rank: Some(rank % 2),
        status: Some("running".into()),
        timestamp: now_micros(),
        ..Default::default()
    }
}

async fn put_nodes_handler(
    State(state): State<AppState>,
    Json(body): Json<NodeReportRequest>,
) -> Json<NodeReportResponse> {
    let version_before = state.version.load(Ordering::Relaxed);
    let seen_version = body.seen_version;
    let incoming = body.nodes.clone();
    let mut cluster = lock_mutex(&state.cluster, "hierarchical_cluster_report cluster");
    for mut node in body.nodes {
        if node.timestamp == 0 {
            node.timestamp = now_micros();
        }
        if node.status.is_none() {
            node.status = Some("running".into());
        }
        cluster.put(node);
    }
    let version = state.version.fetch_add(1, Ordering::Relaxed) + 1;
    let nodes = if seen_version >= version_before {
        incoming
    } else {
        vec![]
    };
    Json(NodeReportResponse {
        ok: true,
        version,
        nodes,
        removed: vec![],
    })
}

async fn get_nodes_handler(State(state): State<AppState>) -> Json<NodeListResponse> {
    let cluster = lock_mutex(&state.cluster, "hierarchical_cluster_report cluster");
    let mut nodes = cluster.list();
    nodes.sort_by_key(|n| n.rank.unwrap_or(i32::MAX));
    let version = state.version.load(Ordering::Relaxed);
    Json(NodeListResponse {
        version,
        total: nodes.len(),
        offset: 0,
        nodes,
    })
}

async fn spawn_cluster_server() -> String {
    let state = AppState {
        cluster: Arc::new(Mutex::new(Cluster::default())),
        version: Arc::new(AtomicU64::new(0)),
    };
    let app = Router::new()
        .route("/apis/nodes", get(get_nodes_handler).put(put_nodes_handler))
        .with_state(state);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

async fn query_handler() -> Json<Message<QueryDataFormat>> {
    Json(Message::new(QueryDataFormat::DataFrame(
        DataFrame::default(),
    )))
}

async fn cluster_query_handler() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "dataframe": DataFrame::default(),
        "meta": {
            "cluster": true,
            "hierarchical": true,
            "scope": "node",
            "nodes_queried": 1,
            "nodes_failed": [],
            "peer_batches_dropped": 0,
            "node_aggregators_queried": 0,
            "local_ranks_queried": 1,
            "partial": false
        }
    }))
}

async fn spawn_authenticated_cluster_server() -> String {
    let state = AppState {
        cluster: Arc::new(Mutex::new(Cluster::default())),
        version: Arc::new(AtomicU64::new(0)),
    };
    let app = Router::new()
        .route("/apis/nodes", get(get_nodes_handler).put(put_nodes_handler))
        .route("/query", post(query_handler))
        .route("/apis/cluster/query", post(cluster_query_handler))
        .with_state(state)
        .layer(middleware::from_fn(selective_auth_middleware));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

/// Mirrors production ``local_leaf_nodes`` for aggregator simulation.
fn aggregator_payload(store: &[Node], self_node: Node) -> Vec<Node> {
    let group_rank = self_node.group_rank.unwrap_or(0);
    let self_rank = self_node.rank.unwrap_or(0);
    let leaves: Vec<Node> = store
        .iter()
        .filter(|n| n.group_rank == Some(group_rank) && n.rank != Some(self_rank))
        .cloned()
        .collect();
    let mut nodes = if leaves.is_empty() {
        vec![self_node.clone()]
    } else {
        let mut merged = leaves;
        if !merged.iter().any(|n| n.rank == self_node.rank) {
            merged.insert(0, self_node);
        }
        merged
    };
    nodes.sort_by_key(|n| n.rank.unwrap_or(i32::MAX));
    nodes
}

fn local_group_ranks(store: &[Node], group_rank: i32) -> Vec<i32> {
    let mut ranks: Vec<i32> = store
        .iter()
        .filter(|n| n.group_rank == Some(group_rank))
        .filter_map(|n| n.rank)
        .collect();
    ranks.sort_unstable();
    ranks
}

#[test]
fn hierarchical_two_nodes_times_two_gpus_converges_on_master() {
    let _guard = lock_mutex(&ENV_LOCK, "hierarchical_cluster_report ENV_LOCK");
    if !local_http_available() {
        return;
    }
    for key in ["RANK", "GROUP_RANK", "LOCAL_RANK"] {
        std::env::remove_var(key);
    }

    SERVER_RUNTIME
        .try_block_on(async {
            let master = spawn_cluster_server().await;
            let node1_local0 = spawn_cluster_server().await;

            put_nodes_blocking(&master, vec![test_node(1, 0, "127.0.0.1:9101")], 0)
                .expect("leaf rank1 put");

            put_nodes_blocking(&node1_local0, vec![test_node(3, 1, "127.0.0.1:9103")], 0)
                .expect("leaf rank3 put");

            let node0_store = fetch_nodes_blocking(&master).expect("read node0 local store");
            let rank0 = test_node(0, 0, "127.0.0.1:9100");
            let node0_batch = aggregator_payload(&node0_store, rank0);
            assert_eq!(
                node0_batch
                    .iter()
                    .filter_map(|n| n.rank)
                    .collect::<Vec<_>>(),
                vec![0, 1]
            );
            put_nodes_blocking(&master, node0_batch, 1).expect("rank0 aggregator put");

            let node1_store = fetch_nodes_blocking(&node1_local0).expect("read node1 local store");
            let rank2 = test_node(2, 1, "127.0.0.1:9102");
            let node1_batch = aggregator_payload(&node1_store, rank2);
            assert_eq!(
                node1_batch
                    .iter()
                    .filter_map(|n| n.rank)
                    .collect::<Vec<_>>(),
                vec![2, 3]
            );
            put_nodes_blocking(&master, node1_batch, 2).expect("rank2 aggregator put");

            let snapshot = fetch_nodes_blocking(&master).expect("master snapshot");
            let ranks: Vec<i32> = snapshot.iter().filter_map(|n| n.rank).collect();
            assert_eq!(ranks, vec![0, 1, 2, 3]);

            assert_eq!(local_group_ranks(&snapshot, 0), vec![0, 1]);
        })
        .expect("probing runtime");
}

#[test]
fn authenticated_peer_traffic_supports_heartbeat_and_fanout() {
    let _guard = lock_mutex(&ENV_LOCK, "hierarchical_cluster_report ENV_LOCK");
    if !local_http_available() {
        return;
    }

    SERVER_RUNTIME
        .try_block_on(async {
            probing_server::initialize_engine()
                .await
                .expect("initialize composition root");
            std::env::set_var(AUTH_TOKEN_ENV, "cluster-secret");
            bootstrap_auth_from_env().await;
            std::env::remove_var(AUTH_TOKEN_ENV);
            let base = spawn_authenticated_cluster_server().await;
            let addr = base.trim_start_matches("http://");
            let unauthenticated_url = format!("{base}/apis/nodes");
            let unauthenticated = tokio::task::spawn_blocking(move || {
                ureq::get(&unauthenticated_url)
                    .call()
                    .expect_err("token is required")
            })
            .await
            .expect("join unauthenticated request");
            assert!(matches!(unauthenticated, ureq::Error::StatusCode(401)));

            put_nodes_blocking(&base, vec![test_node(0, 0, "127.0.0.1:9100")], 0)
                .expect("authenticated heartbeat");
            assert_eq!(
                fetch_nodes_blocking(&base)
                    .expect("authenticated node discovery")
                    .len(),
                1
            );

            remote_query_df(addr, "SELECT 1")
                .await
                .expect("authenticated server leaf fan-out");
            let transport = probing_core::ENGINE
                .read()
                .await
                .peer_query_transport()
                .expect("composition root transport");
            ProbeClusterExecutor::execute_remote_for_scope(
                Some(&transport),
                addr,
                "SELECT 1",
                FanoutScope::Flat,
            )
            .expect("authenticated core leaf fan-out");
            ProbeClusterExecutor::execute_remote_for_scope(
                Some(&transport),
                addr,
                "SELECT 1",
                FanoutScope::Coordinator,
            )
            .expect("authenticated hierarchical fan-out");

            persist_auth_token("").await.expect("clear auth token");
        })
        .expect("probing runtime");
}
