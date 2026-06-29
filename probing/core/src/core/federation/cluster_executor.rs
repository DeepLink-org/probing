use std::sync::{LazyLock, Mutex, MutexGuard};
use std::time::Duration;

use datafusion::error::{DataFusionError, Result};
use probing_proto::prelude::{DataFrame, Message, Node, Query, QueryDataFormat};

use crate::core::cluster::{
    hierarchical_metadata_available, local_leaf_peers, node_aggregator_peers,
    remote_peers_excluding_local,
};
use crate::core::federation::fanout_scope::{
    current_fanout_scope, resolve_fanout_scope, FanoutScope,
};

#[cfg(any(test, feature = "test-utils"))]
type RemoteQueryHook = Box<dyn Fn(&str, &str) -> Result<DataFrame> + Send + Sync>;

#[cfg(any(test, feature = "test-utils"))]
static REMOTE_QUERY_HOOK: LazyLock<Mutex<Option<RemoteQueryHook>>> =
    LazyLock::new(|| Mutex::new(None));

/// Install an in-process remote query handler for federation integration tests.
#[cfg(any(test, feature = "test-utils"))]
pub fn set_remote_query_hook(hook: Option<RemoteQueryHook>) {
    *lock_remote_query_hook() = hook;
}

/// Default per-node timeout for remote federated queries (seconds).
const DEFAULT_REMOTE_QUERY_TIMEOUT_SECS: u64 = 2;
/// Env var to override the per-node remote query timeout (seconds).
const REMOTE_QUERY_TIMEOUT_ENV: &str = "PROBING_REMOTE_QUERY_TIMEOUT_SECS";

fn external<E: std::error::Error + Send + Sync + 'static>(err: E) -> DataFusionError {
    DataFusionError::External(Box::new(err))
}

/// Per-node timeout for remote federated queries.
///
/// Defaults to [`DEFAULT_REMOTE_QUERY_TIMEOUT_SECS`]; override via the
/// `PROBING_REMOTE_QUERY_TIMEOUT_SECS` environment variable. A value of `0`
/// (or an unparseable value) falls back to the default.
pub fn remote_query_timeout() -> Duration {
    let secs = std::env::var(REMOTE_QUERY_TIMEOUT_ENV)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|&v| v > 0)
        .unwrap_or(DEFAULT_REMOTE_QUERY_TIMEOUT_SECS);
    Duration::from_secs(secs)
}

/// Outcome of a remote query against a single peer, retaining node identity so
/// callers can tag rows and account for successes/failures.
pub struct RemoteFanoutResult {
    pub addr: String,
    pub host: String,
    pub rank: Option<i32>,
    pub result: Result<DataFrame>,
}

#[derive(Debug, Default, Clone)]
pub struct FanoutStats {
    pub nodes_succeeded: usize,
    pub nodes_failed: Vec<String>,
}

static LAST_FANOUT_STATS: LazyLock<Mutex<FanoutStats>> =
    LazyLock::new(|| Mutex::new(FanoutStats::default()));

fn lock_fanout_stats() -> MutexGuard<'static, FanoutStats> {
    crate::sync::lock_mutex(&LAST_FANOUT_STATS, "LAST_FANOUT_STATS")
}

#[cfg(any(test, feature = "test-utils"))]
fn lock_remote_query_hook() -> MutexGuard<'static, Option<RemoteQueryHook>> {
    crate::sync::lock_mutex(&REMOTE_QUERY_HOOK, "REMOTE_QUERY_HOOK")
}

pub fn reset_fanout_stats() {
    *lock_fanout_stats() = FanoutStats::default();
}

/// Record the fan-out outcome so callers (e.g. cluster fan-out meta) can report
/// how many peers were actually queried and which ones failed.
pub fn set_fanout_stats(stats: FanoutStats) {
    *lock_fanout_stats() = stats;
}

/// Increment the success counter for one peer (concurrency-safe).
///
/// Used by streaming fan-out where each peer partition reports its own outcome.
pub fn record_fanout_success() {
    lock_fanout_stats().nodes_succeeded += 1;
}

/// Record a failed peer (concurrency-safe).
pub fn record_fanout_failure(addr: &str) {
    lock_fanout_stats().nodes_failed.push(addr.to_string());
}

pub fn take_fanout_stats() -> FanoutStats {
    std::mem::take(&mut *lock_fanout_stats())
}

pub struct ProbeClusterExecutor;

impl ProbeClusterExecutor {
    pub fn local_host_label() -> String {
        std::env::var("HOSTNAME")
            .or_else(|_| std::env::var("HOST"))
            .unwrap_or_else(|_| "localhost".into())
    }

    pub fn local_listen_addrs() -> Vec<String> {
        crate::core::cluster::local_listen_addrs()
    }

    pub fn local_addr_label() -> String {
        crate::core::cluster::local_addr_label()
    }

    /// Peer nodes for the active fan-out scope (deduplicated against listen addrs).
    pub fn remote_nodes() -> Vec<Node> {
        Self::remote_nodes_for_scope(current_fanout_scope())
    }

    pub fn remote_nodes_for_scope(scope: FanoutScope) -> Vec<Node> {
        let scope = resolve_fanout_scope(scope);
        match scope {
            FanoutScope::Local => Vec::new(),
            FanoutScope::Flat => remote_peers_excluding_local(),
            FanoutScope::Coordinator => {
                if hierarchical_metadata_available() {
                    node_aggregator_peers()
                } else {
                    log::debug!(
                        "hierarchical fan-out metadata missing; falling back to flat peers"
                    );
                    remote_peers_excluding_local()
                }
            }
            FanoutScope::Node => {
                if hierarchical_metadata_available() {
                    local_leaf_peers()
                } else {
                    remote_peers_excluding_local()
                }
            }
            FanoutScope::Auto => remote_peers_excluding_local(),
        }
    }

    /// Execute `sql` on every peer node concurrently, returning each node's result.
    ///
    /// Requests run in parallel (one OS thread per peer via [`std::thread::scope`]),
    /// so total latency is bounded by the slowest peer rather than the sum of all
    /// peers. Node identity is preserved for row tagging and fan-out accounting.
    pub fn fanout_query_to_peers(sql: &str) -> Vec<RemoteFanoutResult> {
        Self::fanout_query_to_peers_scoped(sql, current_fanout_scope())
    }

    pub fn fanout_query_to_peers_scoped(sql: &str, scope: FanoutScope) -> Vec<RemoteFanoutResult> {
        let nodes = Self::remote_nodes_for_scope(scope);
        if nodes.is_empty() {
            return Vec::new();
        }
        let scope = resolve_fanout_scope(scope);
        std::thread::scope(|s| {
            let handles: Vec<_> = nodes
                .into_iter()
                .map(|node| {
                    s.spawn(move || {
                        let host = if node.host.is_empty() {
                            node.addr.clone()
                        } else {
                            node.host.clone()
                        };
                        let result = Self::execute_remote_scoped(&node.addr, sql, scope);
                        RemoteFanoutResult {
                            addr: node.addr,
                            host,
                            rank: node.rank,
                            result,
                        }
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|handle| {
                    handle.join().unwrap_or_else(|_| RemoteFanoutResult {
                        addr: String::new(),
                        host: String::new(),
                        rank: None,
                        result: Err(DataFusionError::Execution(
                            "remote query thread panicked".into(),
                        )),
                    })
                })
                .collect()
        })
    }

    /// Peer nodes and execution scope for a federated table scan.
    ///
    /// When hierarchical coordinator tier finds no ``local_rank == 0`` aggregators,
    /// falls back to flat peers and plain remote SQL (matches metadata-missing behavior).
    pub fn federated_scan_targets() -> (Vec<Node>, FanoutScope) {
        let resolved = resolve_fanout_scope(current_fanout_scope());
        match resolved {
            FanoutScope::Coordinator => {
                let peers = Self::remote_nodes_for_scope(FanoutScope::Coordinator);
                if peers.is_empty() {
                    log::debug!("federated scan: no node aggregators; falling back to flat peers");
                    (
                        Self::remote_nodes_for_scope(FanoutScope::Flat),
                        FanoutScope::Flat,
                    )
                } else {
                    (peers, FanoutScope::Coordinator)
                }
            }
            FanoutScope::Node => {
                let peers = Self::remote_nodes_for_scope(FanoutScope::Node);
                if peers.is_empty() {
                    log::debug!("federated scan: no local leaf peers; falling back to flat peers");
                    (
                        Self::remote_nodes_for_scope(FanoutScope::Flat),
                        FanoutScope::Flat,
                    )
                } else {
                    (peers, FanoutScope::Node)
                }
            }
            scope => (Self::remote_nodes_for_scope(scope), scope),
        }
    }

    pub fn execute_remote_query(addr: &str, sql: &str) -> Result<DataFrame> {
        Self::execute_remote_for_scope(addr, sql, current_fanout_scope())
    }

    pub fn execute_remote_for_scope(
        addr: &str,
        sql: &str,
        scope: FanoutScope,
    ) -> Result<DataFrame> {
        Self::execute_remote_scoped(addr, sql, scope)
    }

    fn execute_remote_scoped(addr: &str, sql: &str, scope: FanoutScope) -> Result<DataFrame> {
        let scope = resolve_fanout_scope(scope);
        if scope == FanoutScope::Coordinator {
            return Self::execute_remote_node_aggregate(addr, sql);
        }
        Self::execute_remote_plain(addr, sql)
    }

    /// Ask a node aggregator to fan in on-node ranks (``POST /apis/cluster/query``).
    fn execute_remote_node_aggregate(addr: &str, sql: &str) -> Result<DataFrame> {
        #[cfg(any(test, feature = "test-utils"))]
        if let Some(hook) = lock_remote_query_hook().as_ref() {
            return hook(addr, sql);
        }

        let url = format!("http://{addr}/apis/cluster/query");
        let body = serde_json::json!({
            "expr": sql,
            "cluster": true,
            "hierarchical": true,
            "scope": "node",
        });
        let body = serde_json::to_string(&body).map_err(external)?;
        let addr_owned = addr.to_string();
        let response = ureq::post(&url)
            .config()
            .timeout_global(Some(remote_query_timeout()))
            .build()
            .send(body)
            .map_err(external)?;

        let status = response.status().as_u16();
        let text = response.into_body().read_to_string().map_err(external)?;
        if status >= 400 {
            return Err(DataFusionError::Execution(format!(
                "remote node aggregate {addr_owned} failed: HTTP {status}: {text}"
            )));
        }

        let value: serde_json::Value = serde_json::from_str(&text).map_err(external)?;
        if let Some(err) = value.get("error").and_then(|v| v.as_str()) {
            return Err(DataFusionError::Execution(format!(
                "remote node aggregate {addr_owned}: {err}"
            )));
        }
        let df_value = value.get("dataframe").ok_or_else(|| {
            DataFusionError::Execution(format!(
                "remote node aggregate {addr_owned}: missing dataframe"
            ))
        })?;
        serde_json::from_value(df_value.clone()).map_err(external)
    }

    fn execute_remote_plain(addr: &str, sql: &str) -> Result<DataFrame> {
        #[cfg(any(test, feature = "test-utils"))]
        if let Some(hook) = lock_remote_query_hook().as_ref() {
            return hook(addr, sql);
        }

        let url = format!("http://{addr}/query");
        let request = Message::new(Query {
            expr: sql.to_string(),
            ..Default::default()
        });
        let body = serde_json::to_string(&request).map_err(external)?;
        let addr_owned = addr.to_string();
        let response = ureq::post(&url)
            .config()
            .timeout_global(Some(remote_query_timeout()))
            .build()
            .send(body)
            .map_err(external)?;

        let status = response.status().as_u16();
        let text = response.into_body().read_to_string().map_err(external)?;
        if status >= 400 {
            return Err(DataFusionError::Execution(format!(
                "remote query {addr_owned} failed: HTTP {status}: {text}"
            )));
        }

        let msg: Message<QueryDataFormat> = serde_json::from_str(&text).map_err(external)?;
        match msg.payload {
            QueryDataFormat::DataFrame(df) => Ok(df),
            QueryDataFormat::Nil => Ok(DataFrame::default()),
            QueryDataFormat::Error(err) => Err(DataFusionError::Execution(format!(
                "remote query {addr_owned}: {}",
                err.message
            ))),
            QueryDataFormat::TimeSeries(_) => Err(DataFusionError::NotImplemented(
                "remote timeseries query not supported".into(),
            )),
        }
    }
}
