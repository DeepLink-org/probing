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

/// Record another address this process answers on, keeping the ones already known.
///
/// A wildcard bind is registered as `0.0.0.0:<port>`, but the cluster registry
/// advertises a routable form such as `127.0.0.1:<port>`. Without both spellings
/// the coordinator does not recognise its own registry entry: it fans out to
/// itself, records the attempt as a failed peer, and every federated query comes
/// back marked partial.
pub fn add_local_listen_addrs(addrs: Vec<String>) {
    let mut guard = crate::sync::write_rwlock(&LOCAL_LISTEN_ADDRS, "LOCAL_LISTEN_ADDRS");
    let known = guard.get_or_insert_with(Vec::new);
    for addr in addrs {
        let addr = addr.trim().to_string();
        if !addr.is_empty() && !known.contains(&addr) {
            known.push(addr);
        }
    }
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

/// Whether `addr` binds every interface rather than naming a routable host.
pub fn is_wildcard_addr(addr: &str) -> bool {
    let host = match addr.rsplit_once(':') {
        Some((host, _)) => host.trim().trim_matches(['[', ']']),
        None => return false,
    };
    matches!(host, "0.0.0.0" | "::" | "" | "*")
}

/// The address to label this process with in federated results.
///
/// Wildcard forms are skipped: they never match a registry entry, which would
/// leave the local rows tagged `_rank = -1`.
pub fn local_addr_label() -> String {
    let addrs = local_listen_addrs();
    addrs
        .iter()
        .find(|addr| !is_wildcard_addr(addr))
        .or_else(|| addrs.first())
        .cloned()
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

fn node_key(node: &Node) -> String {
    format!("{}:{}", node.host, node.addr)
}

fn delta_nodes(incoming: &[Node], removed: &[String]) -> Vec<Node> {
    let cluster = read_cluster();
    let mut delta = Vec::with_capacity(incoming.len() + removed.len());
    for node in incoming {
        let key = node_key(node);
        if let Some(stored) = cluster.nodes.get(&key) {
            delta.push(stored.clone());
        } else {
            delta.push(node.clone());
        }
    }
    for key in removed {
        if let Some(node) = cluster.nodes.get(key) {
            delta.push(node.clone());
        }
    }
    delta
}

fn replaced_node_keys(incoming: &[Node]) -> Vec<String> {
    let cluster = read_cluster();
    incoming
        .iter()
        .filter_map(|node| {
            let rank = node.rank?;
            let previous = cluster.get(rank)?;
            let previous_key = node_key(previous);
            (previous_key != node_key(node)).then_some(previous_key)
        })
        .collect()
}

/// Merge reported nodes, sweep stale entries, bump version, return snapshot.
pub fn apply_node_report(incoming: Vec<Node>, seen_version: u64) -> NodeReportResponse {
    let version_before = cluster_version();
    let mut removed = replaced_node_keys(&incoming);
    for node in incoming.iter().cloned() {
        update_node(node);
    }
    let (newly_dead, expired) = sweep_stale_nodes();
    removed.extend(expired);
    removed.sort();
    removed.dedup();
    let changed = !incoming.is_empty() || !newly_dead.is_empty() || !removed.is_empty();
    let version = if changed {
        CLUSTER_VERSION.fetch_add(1, Ordering::Relaxed) + 1
    } else {
        version_before
    };

    let nodes = if seen_version >= version_before && seen_version > 0 {
        delta_nodes(&incoming, &newly_dead)
    } else if seen_version >= version_before {
        if changed {
            delta_nodes(&incoming, &newly_dead)
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    NodeReportResponse {
        ok: true,
        version,
        nodes,
        removed,
    }
}

/// Apply a delta from a parent coordinator without re-POSTing the full snapshot over HTTP.
pub fn apply_snapshot_delta(nodes: Vec<Node>, removed: &[String], version: u64) {
    for node in nodes {
        update_node(node);
    }
    for key in removed {
        let mut parts = key.splitn(2, ':');
        if let (Some(host), Some(addr)) = (parts.next(), parts.next()) {
            write_cluster().remove_by_addr(host, addr);
        }
    }
    CLUSTER_VERSION.store(version, Ordering::Relaxed);
}

/// Paginated node list; returns empty when ``since_version`` is current.
pub fn get_nodes_page(
    offset: usize,
    limit: usize,
    since_version: Option<u64>,
) -> (u64, usize, Vec<Node>) {
    let version = cluster_version();
    if since_version.is_some_and(|v| v >= version) {
        return (version, read_cluster().nodes.len(), Vec::new());
    }
    let all = get_nodes();
    let total = all.len();
    let page = all.into_iter().skip(offset).take(limit).collect();
    (version, total, page)
}

pub fn cluster_version() -> u64 {
    CLUSTER_VERSION.load(Ordering::Relaxed)
}

/// Mark stale nodes dead, then physically remove nodes that remain dead for another TTL.
fn sweep_stale_nodes() -> (Vec<String>, Vec<String>) {
    let threshold = stale_threshold_micros();
    let now = now_micros();
    let mut newly_dead = Vec::new();
    let mut cluster = write_cluster();
    for (key, node) in cluster.nodes.iter_mut() {
        if node.timestamp == 0 {
            continue;
        }
        let age = now.saturating_sub(node.timestamp);
        if age <= threshold {
            continue;
        }
        if node.status.as_deref() == Some("dead") {
            continue;
        }
        node.status = Some("dead".to_string());
        newly_dead.push(key.clone());
    }

    let expired: Vec<String> = cluster
        .nodes
        .iter()
        .filter(|(_, node)| {
            node.status.as_deref() == Some("dead")
                && now.saturating_sub(node.timestamp) > threshold.saturating_mul(2)
        })
        .filter(|(key, _)| !newly_dead.contains(key))
        .map(|(key, _)| key.clone())
        .collect();
    for key in &expired {
        if let Some(node) = cluster.nodes.remove(key) {
            if let Some(rank) = node.rank {
                if cluster.rank_index.get(&rank) == Some(key) {
                    cluster.rank_index.remove(&rank);
                }
            }
        }
    }
    (newly_dead, expired)
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

fn prefer_node_representative(candidate: &Node, existing: &Node) -> bool {
    match (candidate.local_rank, existing.local_rank) {
        (Some(0), Some(0)) => {
            candidate.rank.unwrap_or(i32::MAX) < existing.rank.unwrap_or(i32::MAX)
        }
        (Some(0), _) => true,
        (_, Some(0)) => false,
        _ => candidate.rank.unwrap_or(i32::MAX) < existing.rank.unwrap_or(i32::MAX),
    }
}

/// Alive peers excluding this process's listen addresses.
pub fn remote_peers_excluding_local() -> Vec<Node> {
    let local_addrs = local_listen_addrs();
    get_nodes()
        .into_iter()
        .filter(is_node_alive)
        .filter(|node| !local_addrs.iter().any(|local| local == &node.addr))
        .collect()
}

/// One node-aggregator endpoint per ``group_rank`` (prefers ``local_rank == 0``).
///
/// Used at the coordinator tier so fan-out is O(nodes) not O(world_size).
pub fn node_aggregator_peers() -> Vec<Node> {
    use std::collections::HashMap;

    let local_addrs = local_listen_addrs();
    let mut by_group: HashMap<i32, Node> = HashMap::new();

    for node in get_nodes().into_iter().filter(is_node_alive) {
        if node.local_rank != Some(0) {
            continue;
        }
        if local_addrs.iter().any(|local| local == &node.addr) {
            continue;
        }
        let Some(group_rank) = node.group_rank else {
            continue;
        };
        by_group
            .entry(group_rank)
            .and_modify(|existing| {
                if prefer_node_representative(&node, existing) {
                    *existing = node.clone();
                }
            })
            .or_insert(node);
    }

    let mut peers: Vec<Node> = by_group.into_values().collect();
    peers.sort_by_key(|n| n.group_rank.unwrap_or(i32::MAX));
    peers
}

/// Leaf ranks on this node (same ``group_rank``, excluding self).
pub fn local_leaf_peers() -> Vec<Node> {
    let local_addrs = local_listen_addrs();
    let group_rank = env_i32("GROUP_RANK")
        .or_else(|| env_i32("NODE_RANK"))
        .or_else(|| {
            get_nodes()
                .into_iter()
                .find(|node| local_addrs.iter().any(|local| local == &node.addr))
                .and_then(|node| node.group_rank)
        });
    let Some(group_rank) = group_rank else {
        log::warn!(
            "hierarchical fan-out cannot identify the local group_rank; refusing cross-node leaf selection"
        );
        return Vec::new();
    };
    let self_rank = env_i32("RANK");

    get_nodes()
        .into_iter()
        .filter(is_node_alive)
        .filter(|node| {
            if local_addrs.iter().any(|local| local == &node.addr) {
                return false;
            }
            if node.group_rank != Some(group_rank) {
                return false;
            }
            if let (Some(r), Some(self_r)) = (node.rank, self_rank) {
                if r == self_r {
                    return false;
                }
            }
            true
        })
        .collect()
}

fn env_i32(name: &str) -> Option<i32> {
    std::env::var(name).ok().and_then(|v| v.trim().parse().ok())
}

/// Whether ``cluster.nodes`` has enough metadata for hierarchical fan-out.
pub fn hierarchical_metadata_available() -> bool {
    let alive: Vec<Node> = get_nodes().into_iter().filter(is_node_alive).collect();
    !alive.is_empty()
        && alive
            .iter()
            .all(|n| n.group_rank.is_some() && n.local_rank.is_some())
}

/// Error prefix returned when hierarchical fan-out is requested but metadata is missing.
pub const HIERARCHICAL_METADATA_UNAVAILABLE: &str =
    "cluster hierarchical fan-out unavailable: cluster.nodes missing group_rank/local_rank metadata";

/// Build a user-facing error for missing hierarchical metadata.
pub fn hierarchical_metadata_unavailable_err() -> crate::core::error::EngineError {
    crate::core::error::EngineError::ClusterError(format!(
        "{HIERARCHICAL_METADATA_UNAVAILABLE} — wait for torchrun heartbeat to converge, \
         or set hierarchical=false / PROBING_CLUSTER_FANOUT_HIERARCHICAL=0 for legacy flat fan-out"
    ))
}

pub fn is_hierarchical_metadata_unavailable(err: &impl std::fmt::Display) -> bool {
    err.to_string()
        .starts_with(HIERARCHICAL_METADATA_UNAVAILABLE)
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
    fn wildcard_addrs_are_recognised() {
        assert!(is_wildcard_addr("0.0.0.0:18080"));
        assert!(is_wildcard_addr("[::]:18080"));
        assert!(is_wildcard_addr(":18080"));
        assert!(!is_wildcard_addr("127.0.0.1:18080"));
        assert!(!is_wildcard_addr("10.0.0.5:18080"));
    }

    #[test]
    fn advertised_addr_is_kept_alongside_the_bound_one() {
        let _guard = TEST_LOCK.lock().unwrap();
        set_local_listen_addrs(vec!["0.0.0.0:40473".into()]);
        add_local_listen_addrs(vec!["127.0.0.1:40473".into()]);
        add_local_listen_addrs(vec!["127.0.0.1:40473".into()]);

        let addrs = local_listen_addrs();
        assert_eq!(addrs, vec!["0.0.0.0:40473", "127.0.0.1:40473"]);
        // A wildcard label never matches a registry entry, so `_rank` would be -1.
        assert_eq!(local_addr_label(), "127.0.0.1:40473");

        set_local_listen_addrs(vec![]);
    }

    #[test]
    fn coordinator_is_not_its_own_peer() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_cluster_for_tests();
        set_local_listen_addrs(vec!["0.0.0.0:40473".into()]);
        add_local_listen_addrs(vec!["127.0.0.1:40473".into()]);

        // How the registry sees this process: the advertised, routable form.
        update_node(Node {
            host: "h1".into(),
            addr: "127.0.0.1:40473".into(),
            rank: Some(0),
            status: Some("running".into()),
            ..Default::default()
        });
        update_node(Node {
            host: "h1".into(),
            addr: "127.0.0.1:33241".into(),
            rank: Some(1),
            status: Some("running".into()),
            ..Default::default()
        });

        let peers = remote_peers_excluding_local();
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].addr, "127.0.0.1:33241");

        set_local_listen_addrs(vec![]);
        reset_cluster_for_tests();
    }

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
        let resp = apply_node_report(vec![], cluster_version());
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
        let resp = apply_node_report(vec![node], 0);
        assert_eq!(resp.nodes.len(), 1);
        assert_eq!(resp.nodes[0].status.as_deref(), Some("running"));
        assert!(resp.nodes[0].timestamp > 0);
    }

    #[test]
    fn incremental_heartbeat_returns_delta_not_full_snapshot() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_cluster_for_tests();
        let node = Node {
            host: "h1".into(),
            addr: "10.0.0.1:3".into(),
            rank: Some(3),
            status: Some("running".into()),
            ..Default::default()
        };
        let first = apply_node_report(vec![node.clone()], 0);
        assert_eq!(first.nodes.len(), 1);
        let v = first.version;
        let second = apply_node_report(vec![node], v);
        assert_eq!(second.nodes.len(), 1);
        assert!(second.version >= v);
        let third = apply_node_report(vec![], second.version);
        assert!(third.nodes.is_empty());
    }

    #[test]
    fn get_nodes_page_respects_since_version() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_cluster_for_tests();
        write_cluster().put(Node {
            host: "h".into(),
            addr: "10.0.0.1:0".into(),
            rank: Some(0),
            ..Default::default()
        });
        let version = cluster_version();
        let (_, total, page) = get_nodes_page(0, 10, Some(version));
        assert_eq!(total, 1);
        assert!(page.is_empty());
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

    #[test]
    fn node_aggregator_peers_one_per_group_rank() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_cluster_for_tests();
        set_local_listen_addrs(vec!["127.0.0.1:9000".into()]);
        write_cluster().put(Node {
            host: "h".into(),
            addr: "127.0.0.1:9000".into(),
            rank: Some(0),
            group_rank: Some(0),
            local_rank: Some(0),
            status: Some("running".into()),
            ..Default::default()
        });
        for (rank, group, local) in [(1, 0, 1), (8, 1, 0), (9, 1, 1)] {
            let port = 8080 + rank;
            write_cluster().put(Node {
                host: "h".into(),
                addr: format!("10.0.0.{rank}:{port}"),
                rank: Some(rank),
                group_rank: Some(group),
                local_rank: Some(local),
                status: Some("running".into()),
                ..Default::default()
            });
        }
        let peers = node_aggregator_peers();
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].rank, Some(8));
        assert_eq!(peers[0].local_rank, Some(0));
    }

    #[test]
    fn local_leaf_peers_same_group_only() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_cluster_for_tests();
        set_local_listen_addrs(vec!["127.0.0.1:9000".into()]);
        std::env::set_var("GROUP_RANK", "0");
        std::env::set_var("RANK", "0");
        for (rank, group, local) in [(0, 0, 0), (1, 0, 1), (8, 1, 0)] {
            let port = 8080 + rank;
            write_cluster().put(Node {
                host: "h".into(),
                addr: format!("10.0.0.{rank}:{port}"),
                rank: Some(rank),
                group_rank: Some(group),
                local_rank: Some(local),
                status: Some("running".into()),
                ..Default::default()
            });
        }
        let leaves = local_leaf_peers();
        assert_eq!(leaves.len(), 1);
        assert_eq!(leaves[0].rank, Some(1));
        std::env::remove_var("GROUP_RANK");
        std::env::remove_var("RANK");
    }

    #[test]
    fn hierarchical_metadata_requires_every_alive_node() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_cluster_for_tests();
        for (rank, group_rank, local_rank) in [(0, Some(0), Some(0)), (1, None, Some(1))] {
            write_cluster().put(Node {
                host: "h".into(),
                addr: format!("10.0.0.1:{}", 8080 + rank),
                rank: Some(rank),
                group_rank,
                local_rank,
                status: Some("running".into()),
                ..Default::default()
            });
        }

        assert!(!hierarchical_metadata_available());
        reset_cluster_for_tests();
    }
}
