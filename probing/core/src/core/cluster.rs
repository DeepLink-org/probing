use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, RwLock, RwLockReadGuard, RwLockWriteGuard};

use arrow::array::{ArrayRef, Int32Array, StringArray, TimestampMicrosecondArray};
use probing_proto::prelude::{Cluster, Node, NodeReportResponse};

pub trait IntoArrow {
    fn into_arrow_array(values: Vec<Self>) -> ArrayRef
    where
        Self: Sized;
}

// Implementation for String
impl IntoArrow for String {
    fn into_arrow_array(values: Vec<Self>) -> ArrayRef {
        Arc::new(StringArray::from(values))
    }
}

impl IntoArrow for Option<String> {
    fn into_arrow_array(values: Vec<Self>) -> ArrayRef {
        Arc::new(StringArray::from(values))
    }
}

impl IntoArrow for Option<i32> {
    fn into_arrow_array(values: Vec<Self>) -> ArrayRef {
        Arc::new(Int32Array::from(values))
    }
}

impl IntoArrow for std::time::Duration {
    fn into_arrow_array(values: Vec<Self>) -> ArrayRef {
        Arc::new(TimestampMicrosecondArray::from(
            values
                .iter()
                .map(|v| Some(v.as_micros() as i64))
                .collect::<Vec<_>>(),
        ))
    }
}

pub fn extract_array<T, V, F>(nodes: &[T], f: F) -> ArrayRef
where
    F: FnMut(&T) -> V,
    V: IntoArrow,
{
    let values: Vec<V> = nodes.iter().map(f).collect();
    V::into_arrow_array(values)
}

pub static CLUSTER: LazyLock<RwLock<Cluster>> = LazyLock::new(|| RwLock::new(Cluster::default()));

static LOCAL_LISTEN_ADDRS: LazyLock<RwLock<Option<Vec<String>>>> =
    LazyLock::new(|| RwLock::new(None));

static CLUSTER_VERSION: AtomicU64 = AtomicU64::new(0);

fn now_micros() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(0)
}

fn stale_threshold_micros() -> u64 {
    std::env::var("PROBING_CLUSTER_STALE_SEC")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(25)
        * 1_000_000
}

/// Override local listen address(es) at runtime (e.g. when the HTTP server binds).
pub fn set_local_listen_addrs(addrs: Vec<String>) {
    *crate::sync::write_rwlock(&LOCAL_LISTEN_ADDRS, "LOCAL_LISTEN_ADDRS") = Some(addrs);
}

/// Local listen address(es): runtime override, then `PROBING_ADDRESS` env, then default.
pub fn local_listen_addrs() -> Vec<String> {
    if let Some(addrs) =
        crate::sync::read_rwlock(&LOCAL_LISTEN_ADDRS, "LOCAL_LISTEN_ADDRS").as_ref()
    {
        if !addrs.is_empty() {
            return addrs.clone();
        }
    }
    if let Ok(addr) = std::env::var("PROBING_ADDRESS") {
        if !addr.trim().is_empty() {
            return vec![addr];
        }
    }
    vec!["127.0.0.1:8080".into()]
}

pub fn local_addr_label() -> String {
    local_listen_addrs()
        .into_iter()
        .next()
        .unwrap_or_else(|| "127.0.0.1:8080".into())
}

fn read_cluster() -> RwLockReadGuard<'static, Cluster> {
    crate::sync::read_rwlock(&CLUSTER, "CLUSTER")
}

fn write_cluster() -> RwLockWriteGuard<'static, Cluster> {
    crate::sync::write_rwlock(&CLUSTER, "CLUSTER")
}

pub fn update_node(mut node: Node) {
    node.timestamp = now_micros();
    if node.status.is_none() {
        node.status = Some("running".to_string());
    }
    write_cluster().put(node);
}

pub fn update_nodes(nodes: Vec<Node>) {
    for node in nodes {
        update_node(node);
    }
}

/// Merge reported nodes, sweep stale entries to ``dead``, bump version, return snapshot.
pub fn apply_node_report(incoming: Vec<Node>) -> NodeReportResponse {
    for node in incoming {
        update_node(node);
    }
    let removed = mark_stale_nodes_as_dead();
    let version = CLUSTER_VERSION.fetch_add(1, Ordering::Relaxed) + 1;
    NodeReportResponse {
        ok: true,
        version,
        nodes: get_nodes(),
        removed,
    }
}

pub fn cluster_version() -> u64 {
    CLUSTER_VERSION.load(Ordering::Relaxed)
}

/// Mark nodes whose heartbeat timestamp is older than the stale threshold as ``dead``.
fn mark_stale_nodes_as_dead() -> Vec<String> {
    let threshold = stale_threshold_micros();
    let now = now_micros();
    let mut newly_dead = Vec::new();
    let mut cluster = write_cluster();
    for (key, node) in cluster.nodes.iter_mut() {
        if node.timestamp == 0 {
            continue;
        }
        if now.saturating_sub(node.timestamp) <= threshold {
            continue;
        }
        if node.status.as_deref() == Some("dead") {
            continue;
        }
        node.status = Some("dead".to_string());
        newly_dead.push(key.clone());
    }
    newly_dead
}

pub fn is_node_alive(node: &Node) -> bool {
    node.status.as_deref() != Some("dead")
}

pub fn get_nodes() -> Vec<Node> {
    let mut nodes = read_cluster().list();
    nodes.sort_by(|a, b| match (a.rank, b.rank) {
        (Some(ra), Some(rb)) => ra.cmp(&rb),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.addr.cmp(&b.addr),
    });
    nodes
}

#[cfg(any(test, feature = "test-utils"))]
pub fn reset_cluster_for_tests() {
    *write_cluster() = Cluster::default();
    CLUSTER_VERSION.store(0, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use std::sync::{LazyLock, Mutex};

    use super::*;

    static TEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    #[test]
    fn apply_report_marks_stale_nodes_dead() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_cluster_for_tests();
        let old = Node {
            host: "h1".into(),
            addr: "10.0.0.1:1".into(),
            rank: Some(1),
            status: Some("running".into()),
            timestamp: now_micros().saturating_sub(60 * 1_000_000),
            ..Default::default()
        };
        write_cluster().put(old);
        let resp = apply_node_report(vec![]);
        assert!(resp.nodes.iter().any(|n| n.rank == Some(1)));
        let dead = resp.nodes.iter().find(|n| n.rank == Some(1)).unwrap();
        assert_eq!(dead.status.as_deref(), Some("dead"));
        assert!(resp.version >= 1);
    }

    #[test]
    fn apply_report_refreshes_running_node() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_cluster_for_tests();
        let node = Node {
            host: "h1".into(),
            addr: "10.0.0.1:2".into(),
            rank: Some(2),
            status: Some("running".into()),
            ..Default::default()
        };
        let resp = apply_node_report(vec![node]);
        assert_eq!(resp.nodes.len(), 1);
        assert_eq!(resp.nodes[0].status.as_deref(), Some("running"));
        assert!(resp.nodes[0].timestamp > 0);
    }

    #[test]
    fn get_nodes_sorted_by_rank() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_cluster_for_tests();
        for rank in [3, 0, 2, 1] {
            write_cluster().put(Node {
                host: "h".into(),
                addr: format!("10.0.0.1:{rank}"),
                rank: Some(rank),
                ..Default::default()
            });
        }
        let ranks: Vec<i32> = get_nodes().into_iter().filter_map(|n| n.rank).collect();
        assert_eq!(ranks, vec![0, 1, 2, 3]);
    }
}
