//! Hierarchical cluster report integration test (mock HTTP servers + PUT merge path).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::{extract::State, routing::get, Json, Router};
use probing_core::sync::lock_mutex;
use probing_proto::prelude::{
    Cluster, Node, NodeListResponse, NodeReportRequest, NodeReportResponse,
};
use probing_server::cluster_http::{fetch_nodes_blocking, put_nodes_blocking};
use probing_server::server::SERVER_RUNTIME;
use tokio::net::TcpListener;

#[derive(Clone)]
struct AppState {
    cluster: Arc<Mutex<Cluster>>,
    version: Arc<AtomicU64>,
}

static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

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
    for key in ["RANK", "GROUP_RANK", "LOCAL_RANK"] {
        std::env::remove_var(key);
    }

    SERVER_RUNTIME.block_on(async {
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
    });
}
