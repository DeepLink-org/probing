//! Hierarchical torchrun cluster heartbeat (side channel; does not touch torch rendezvous keys).
//!
//! Keys live under ``probing/torchrun/<run_id>/`` on the job TCPStore.
//!
//! Heartbeat interval uses exponential backoff when membership is stable (see
//! ``cluster_report_backoff``). Env:
//! - ``PROBING_CLUSTER_REPORT_INTERVAL_SEC``: base interval (default 10)
//! - ``PROBING_CLUSTER_REPORT_MAX_INTERVAL_SEC``: backoff cap (default 120, clamped below stale TTL)
//! - ``PROBING_CLUSTER_REPORT_BACKOFF_FACTOR``: multiplier per stable tick (default 2)
//! - ``PROBING_CLUSTER_REPORT_BACKOFF``: set ``0`` to disable backoff
//! - ``PROBING_CLUSTER_STALE_SEC``: node dead threshold (default 25; should exceed max interval)

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use probing_proto::prelude::Node;
use probing_store::store::TCPStore;
use serde::{Deserialize, Serialize};

use crate::cluster_http::{fetch_nodes_blocking, get_i32_env, put_nodes_blocking};
use crate::cluster_report_backoff::{
    base_report_interval_secs, classify_report_outcome, ReportBackoff, ReportOutcome,
};
use crate::report::{build_local_node, get_hostname};
use crate::server::SERVER_RUNTIME;
use crate::start_remote;
use crate::vars::read_probing_address;

static STARTED: AtomicBool = AtomicBool::new(false);
static CLUSTER_VERSION: AtomicU64 = AtomicU64::new(0);

static MASTER_INFO: LazyLock<Mutex<Option<StoreHttpInfo>>> = LazyLock::new(|| Mutex::new(None));
static LOCAL0_PARENT: LazyLock<Mutex<Option<String>>> = LazyLock::new(|| Mutex::new(None));
static REPORT_BACKOFF: LazyLock<Mutex<ReportBackoff>> =
    LazyLock::new(|| Mutex::new(ReportBackoff::new()));

#[derive(Debug, Clone, Deserialize, Serialize)]
struct StoreHttpInfo {
    addr: String,
    http_base: String,
    #[serde(default)]
    bound: String,
}

fn torchrun_cluster_enabled() -> bool {
    match std::env::var("PROBING_TORCHRUN_CLUSTER") {
        Ok(val) => {
            let lower = val.trim().to_ascii_lowercase();
            !matches!(lower.as_str(), "0" | "false" | "no")
        }
        Err(_) => true,
    }
}

fn cluster_report_enabled() -> bool {
    match std::env::var("PROBING_CLUSTER_REPORT") {
        Ok(val) => {
            let lower = val.trim().to_ascii_lowercase();
            !matches!(lower.as_str(), "0" | "false" | "no" | "off")
        }
        Err(_) => true,
    }
}

fn discover_timeout_secs() -> f64 {
    std::env::var("PROBING_CLUSTER_DISCOVER_TIMEOUT_SEC")
        .ok()
        .and_then(|v| v.trim().parse::<f64>().ok())
        .unwrap_or(2.0)
        .max(0.1_f64)
}

fn is_elastic_supervisor() -> bool {
    if std::env::var("LOCAL_RANK").is_ok() || std::env::var("RANK").is_ok() {
        return false;
    }
    if std::env::var("TORCHELASTIC_RUN_ID").is_ok() {
        return true;
    }
    std::env::args().any(|arg| {
        let a = arg.as_str();
        a.ends_with("torchrun") || a.contains("torch/distributed/run")
    })
}

fn world_size() -> i32 {
    get_i32_env("WORLD_SIZE").unwrap_or(1)
}

fn global_rank() -> i32 {
    get_i32_env("RANK").unwrap_or(0)
}

fn local_rank() -> i32 {
    get_i32_env("LOCAL_RANK").unwrap_or(0)
}

fn is_global_rank0() -> bool {
    global_rank() == 0
}

fn node_rank() -> i32 {
    get_i32_env("GROUP_RANK")
        .or_else(|| get_i32_env("NODE_RANK"))
        .unwrap_or(0)
}

fn run_prefix() -> String {
    let run_id = std::env::var("TORCHELASTIC_RUN_ID")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("RDZV_ID").ok().filter(|s| !s.is_empty()))
        .or_else(|| std::env::var("MASTER_PORT").ok().filter(|s| !s.is_empty()))
        .unwrap_or_else(|| "run".to_string());
    format!("probing/torchrun/{run_id}")
}

fn master_store_key() -> String {
    format!("{}/master", run_prefix())
}

fn local0_store_key() -> String {
    format!("{}/node/{}/local0", run_prefix(), node_rank())
}

fn rendezvous_endpoint() -> Option<String> {
    let host = std::env::var("MASTER_ADDR")
        .ok()
        .filter(|s| !s.is_empty())?;
    let port = std::env::var("MASTER_PORT")
        .ok()
        .filter(|s| !s.is_empty())?;
    Some(format!("{host}:{port}"))
}

fn bind_spec() -> String {
    if is_global_rank0() {
        if let Ok(port) = std::env::var("PROBING_PORT") {
            let port = port.trim();
            if !port.is_empty()
                && !port.eq_ignore_ascii_case("random")
                && port.parse::<u16>().is_ok()
            {
                return format!("0.0.0.0:{port}");
            }
        }
    }
    "0.0.0.0:0".to_string()
}

fn reachable_addr(bound: &str) -> String {
    let Some((host, port)) = bound.rsplit_once(':') else {
        return bound.to_string();
    };
    let host = host.trim().trim_matches(['[', ']']);
    if matches!(host, "0.0.0.0" | "::" | "" | "*") {
        let master = std::env::var("MASTER_ADDR").unwrap_or_default();
        let master = master.trim();
        if master == "127.0.0.1" || master == "localhost" {
            return format!("{master}:{port}");
        }
        if !master.is_empty() {
            return format!("{master}:{port}");
        }
        return format!(
            "{}:{port}",
            get_hostname().unwrap_or_else(|_| "localhost".into())
        );
    }
    format!("{host}:{port}")
}

fn local_http_base() -> String {
    let bound = read_probing_address().clone();
    format!("http://{}", reachable_addr(&bound))
}

fn parallel_role_from_env() -> Option<String> {
    std::env::var("PROBING_NODE_ROLE")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().to_string())
}

fn build_torchrun_node() -> Node {
    let bound = read_probing_address().clone();
    let mut node = build_local_node(&bound);
    node.addr = reachable_addr(&node.addr);
    node.role = parallel_role_from_env();
    node
}

fn store_client() -> Option<TCPStore> {
    rendezvous_endpoint().map(|ep| TCPStore::new(ep).with_key_prefix(""))
}

async fn store_set(key: &str, value: &str) -> Result<()> {
    let store = store_client().context("MASTER_ADDR/MASTER_PORT not set")?;
    store
        .set(key, value)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))
}

async fn store_get(key: &str) -> Result<Option<String>> {
    let store = store_client().context("MASTER_ADDR/MASTER_PORT not set")?;
    match store.get(key).await {
        Ok(v) => Ok(Some(v)),
        Err(e) => {
            log::debug!("torchrun cluster store get {key}: {e}");
            Ok(None)
        }
    }
}

async fn wait_for_local_address(timeout: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    while tokio::time::Instant::now() < deadline {
        if read_probing_address().contains(':') {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

async fn publish_master() -> Result<()> {
    if !is_global_rank0() {
        return Ok(());
    }
    let bound = read_probing_address().clone();
    let reachable = reachable_addr(&bound);
    let info = StoreHttpInfo {
        addr: reachable.clone(),
        http_base: format!("http://{reachable}"),
        bound,
    };
    store_set(
        &master_store_key(),
        &serde_json::to_string(&info).context("serialize master info")?,
    )
    .await?;
    *MASTER_INFO.lock().unwrap() = Some(info);
    log::info!(
        "probing torchrun: published master at {} (key={})",
        reachable,
        master_store_key()
    );
    Ok(())
}

async fn publish_local0() -> Result<()> {
    if local_rank() != 0 {
        return Ok(());
    }
    let bound = read_probing_address().clone();
    let reachable = reachable_addr(&bound);
    let info = serde_json::json!({
        "addr": reachable,
        "http_base": format!("http://{reachable}"),
        "bound": bound,
        "node_rank": node_rank(),
    });
    store_set(&local0_store_key(), &info.to_string()).await?;
    log::info!(
        "probing torchrun: published local0 at {} (key={})",
        reachable,
        local0_store_key()
    );
    Ok(())
}

async fn poll_master_info(timeout: Duration) -> Option<StoreHttpInfo> {
    if is_global_rank0() {
        let bound = read_probing_address().clone();
        let reachable = reachable_addr(&bound);
        return Some(StoreHttpInfo {
            addr: reachable.clone(),
            http_base: format!("http://{reachable}"),
            bound,
        });
    }
    let deadline = tokio::time::Instant::now() + timeout;
    let key = master_store_key();
    while tokio::time::Instant::now() < deadline {
        if let Ok(Some(raw)) = store_get(&key).await {
            if let Ok(info) = serde_json::from_str::<StoreHttpInfo>(&raw) {
                if !info.http_base.is_empty() {
                    log::info!(
                        "probing torchrun: rank {} discovered master {}",
                        global_rank(),
                        info.addr
                    );
                    return Some(info);
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    None
}

async fn poll_local0_parent(timeout: Duration) -> Option<String> {
    if local_rank() == 0 {
        return Some(local_http_base());
    }
    let deadline = tokio::time::Instant::now() + timeout;
    let key = local0_store_key();
    while tokio::time::Instant::now() < deadline {
        if let Ok(Some(raw)) = store_get(&key).await {
            if let Ok(info) = serde_json::from_str::<StoreHttpInfo>(&raw) {
                if !info.http_base.is_empty() {
                    return Some(info.http_base);
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    None
}

fn local_group_nodes(nodes: &[Node]) -> Vec<Node> {
    let grp = node_rank();
    nodes
        .iter()
        .filter(|n| n.group_rank == Some(grp))
        .cloned()
        .collect()
}

fn local_leaf_nodes(nodes: &[Node]) -> Vec<Node> {
    let self_rank = global_rank();
    local_group_nodes(nodes)
        .into_iter()
        .filter(|n| n.rank != Some(self_rank))
        .collect()
}

fn merge_self_into_nodes(mut nodes: Vec<Node>) -> Vec<Node> {
    let self_node = build_torchrun_node();
    let self_rank = self_node.rank;
    if let Some(rank) = self_rank {
        if nodes.iter().any(|n| n.rank == Some(rank)) {
            return nodes;
        }
    }
    nodes.insert(0, self_node);
    nodes
}

fn nodes_for_upstream_report() -> Vec<Node> {
    if local_rank() != 0 {
        return vec![build_torchrun_node()];
    }
    let local_base = local_http_base();
    let store_nodes = fetch_nodes_blocking(&local_base).unwrap_or_default();
    let leaves = local_leaf_nodes(&store_nodes);
    if leaves.is_empty() {
        vec![build_torchrun_node()]
    } else {
        merge_self_into_nodes(leaves)
    }
}

fn report_parent_http() -> Option<String> {
    if local_rank() != 0 {
        return LOCAL0_PARENT.lock().unwrap().clone();
    }
    if is_global_rank0() {
        return Some(local_http_base());
    }
    MASTER_INFO
        .lock()
        .unwrap()
        .as_ref()
        .map(|m| m.http_base.clone())
}

fn ensure_master_info() -> bool {
    if MASTER_INFO.lock().unwrap().is_some() {
        return true;
    }
    let timeout = Duration::from_secs_f64(discover_timeout_secs());
    if let Some(info) = SERVER_RUNTIME.block_on(poll_master_info(timeout)) {
        *MASTER_INFO.lock().unwrap() = Some(info);
        return true;
    }
    false
}

fn ensure_local0_parent() -> bool {
    if local_rank() == 0 {
        return true;
    }
    if LOCAL0_PARENT.lock().unwrap().is_some() {
        return true;
    }
    let timeout = Duration::from_secs_f64(discover_timeout_secs());
    if let Some(base) = SERVER_RUNTIME.block_on(poll_local0_parent(timeout)) {
        *LOCAL0_PARENT.lock().unwrap() = Some(base);
        return true;
    }
    false
}

fn apply_global_snapshot(resp: &probing_proto::prelude::NodeReportResponse) {
    if resp.nodes.is_empty() {
        return;
    }
    let local_base = local_http_base();
    let version = resp.version;
    if let Err(err) = put_nodes_blocking(&local_base, resp.nodes.clone(), version) {
        log::debug!("probing torchrun: apply global snapshot failed: {err}");
    } else {
        CLUSTER_VERSION.store(version, Ordering::Relaxed);
    }
}

fn report_once() -> ReportOutcome {
    if !ensure_master_info() || !ensure_local0_parent() {
        return ReportOutcome::Skipped;
    }
    let Some(parent) = report_parent_http() else {
        return ReportOutcome::Skipped;
    };
    let nodes = nodes_for_upstream_report();
    let seen = CLUSTER_VERSION.load(Ordering::Relaxed);
    match put_nodes_blocking(&parent, nodes, seen) {
        Ok(resp) => {
            CLUSTER_VERSION.store(resp.version, Ordering::Relaxed);
            if global_rank() != 0 && local_rank() == 0 && !resp.nodes.is_empty() {
                apply_global_snapshot(&resp);
            }
            classify_report_outcome(true, true, Some(&resp))
        }
        Err(err) => {
            log::warn!("probing torchrun: cluster report failed: {err}");
            ReportOutcome::Failed
        }
    }
}

async fn hierarchical_report_worker() {
    let stagger = (global_rank().min(8) as f64) * 0.15;
    tokio::time::sleep(Duration::from_secs_f64(stagger)).await;

    loop {
        let outcome = tokio::task::spawn_blocking(report_once)
            .await
            .unwrap_or(ReportOutcome::Failed);
        let sleep_for = {
            let mut backoff = REPORT_BACKOFF.lock().unwrap();
            backoff.record(outcome);
            backoff.sleep_duration()
        };
        tokio::time::sleep(sleep_for).await;
    }
}

async fn torchrun_setup() -> Result<()> {
    if !wait_for_local_address(Duration::from_secs(10)).await {
        log::warn!("probing torchrun: HTTP server did not bind in time");
        return Ok(());
    }

    publish_master().await.ok();
    publish_local0().await.ok();

    if !cluster_report_enabled() {
        log::info!("probing torchrun: periodic cluster report disabled (PROBING_CLUSTER_REPORT=0)");
        return Ok(());
    }

    log::info!(
        "probing torchrun: hierarchical report worker started (base_interval={}s, max={}s, backoff=on)",
        base_report_interval_secs(),
        crate::cluster_report_backoff::max_report_interval_secs()
    );
    hierarchical_report_worker().await;
    Ok(())
}

/// Whether hierarchical torchrun cluster heartbeat is running.
pub fn is_torchrun_cluster_active() -> bool {
    STARTED.load(Ordering::SeqCst)
}

/// Start HTTP bind + hierarchical cluster heartbeat when ``WORLD_SIZE > 1`` (idempotent).
pub fn maybe_start_torchrun_cluster() {
    if !torchrun_cluster_enabled() || is_elastic_supervisor() || world_size() <= 1 {
        return;
    }
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }

    let bind = bind_spec();
    log::debug!("probing torchrun: binding HTTP at {bind}");
    start_remote(Some(bind));

    SERVER_RUNTIME.spawn(async {
        if let Err(err) = torchrun_setup().await {
            log::warn!("probing torchrun cluster setup failed: {err}");
        }
    });
}

/// Trigger one heartbeat (e.g. after ``set_role``).
pub fn refresh_torchrun_role() -> bool {
    if !STARTED.load(Ordering::SeqCst) {
        return false;
    }
    REPORT_BACKOFF.lock().unwrap().reset();
    SERVER_RUNTIME.spawn(async {
        let outcome = tokio::task::spawn_blocking(report_once)
            .await
            .unwrap_or(ReportOutcome::Failed);
        let mut backoff = REPORT_BACKOFF.lock().unwrap();
        backoff.record(outcome);
    });
    true
}

/// Master HTTP base URL when this process is global rank 0.
pub fn master_http_base() -> Option<String> {
    MASTER_INFO
        .lock()
        .unwrap()
        .as_ref()
        .map(|m| m.http_base.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{LazyLock, Mutex};

    static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    fn clear_torchrun_env() {
        for key in [
            "RDZV_ID",
            "TORCHELASTIC_RUN_ID",
            "MASTER_PORT",
            "NODE_RANK",
            "GROUP_RANK",
            "RANK",
            "LOCAL_RANK",
            "PROBING_PORT",
            "MASTER_ADDR",
        ] {
            std::env::remove_var(key);
        }
    }

    #[test]
    fn run_prefix_uses_rdzv_id() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_torchrun_env();
        std::env::set_var("RDZV_ID", "probing-29680");
        assert_eq!(run_prefix(), "probing/torchrun/probing-29680");
    }

    #[test]
    fn local0_store_key_includes_node_rank() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_torchrun_env();
        std::env::set_var("TORCHELASTIC_RUN_ID", "job-x");
        std::env::set_var("NODE_RANK", "3");
        assert_eq!(local0_store_key(), "probing/torchrun/job-x/node/3/local0");
    }

    #[test]
    fn merge_self_is_idempotent() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_torchrun_env();
        std::env::set_var("RANK", "0");
        std::env::set_var("LOCAL_RANK", "0");
        let self_node = Node {
            rank: Some(0),
            addr: "127.0.0.1:1".into(),
            ..Default::default()
        };
        let nodes = vec![
            self_node.clone(),
            Node {
                rank: Some(1),
                ..Default::default()
            },
        ];
        let merged = merge_self_into_nodes(nodes.clone());
        assert_eq!(merged.len(), 2);
    }

    #[test]
    fn local_group_and_leaf_filters() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_torchrun_env();
        std::env::set_var("GROUP_RANK", "3");
        std::env::set_var("RANK", "24");
        let store: Vec<Node> = (0..40)
            .map(|i| Node {
                rank: Some(i),
                group_rank: Some(i / 8),
                ..Default::default()
            })
            .collect();
        let grp: std::collections::HashSet<_> = local_group_nodes(&store)
            .into_iter()
            .filter_map(|n| n.rank)
            .collect();
        assert_eq!(grp, (24..=31).collect());
        let leaves: std::collections::HashSet<_> = local_leaf_nodes(&store)
            .into_iter()
            .filter_map(|n| n.rank)
            .collect();
        assert_eq!(leaves, (25..=31).collect());
    }

    #[test]
    fn reachable_addr_maps_unspecified_bind() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_torchrun_env();
        std::env::set_var("MASTER_ADDR", "10.0.0.1");
        assert_eq!(reachable_addr("0.0.0.0:9922"), "10.0.0.1:9922");
    }

    #[test]
    fn bind_spec_global_rank0_uses_probing_port() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_torchrun_env();
        std::env::set_var("RANK", "0");
        std::env::set_var("PROBING_PORT", "18080");
        assert_eq!(bind_spec(), "0.0.0.0:18080");
        std::env::set_var("RANK", "1");
        assert_eq!(bind_spec(), "0.0.0.0:0");
    }
}
