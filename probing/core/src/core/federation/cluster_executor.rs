use std::sync::{LazyLock, Mutex, MutexGuard};
use std::time::Duration;

use datafusion::error::{DataFusionError, Result};
use probing_proto::prelude::{DataFrame, Message, Node, Query, QueryDataFormat};

use crate::core::cluster::get_nodes;

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

    /// Peer nodes that are not the local node (deduplicated against listen addrs).
    pub fn remote_nodes() -> Vec<Node> {
        let local_addrs = Self::local_listen_addrs();
        get_nodes()
            .into_iter()
            .filter(crate::core::cluster::is_node_alive)
            .filter(|node| !local_addrs.iter().any(|local| local == &node.addr))
            .collect()
    }

    /// Execute `sql` on every peer node concurrently, returning each node's result.
    ///
    /// Requests run in parallel (one OS thread per peer via [`std::thread::scope`]),
    /// so total latency is bounded by the slowest peer rather than the sum of all
    /// peers. Node identity is preserved for row tagging and fan-out accounting.
    pub fn fanout_query_to_peers(sql: &str) -> Vec<RemoteFanoutResult> {
        let nodes = Self::remote_nodes();
        if nodes.is_empty() {
            return Vec::new();
        }
        std::thread::scope(|scope| {
            let handles: Vec<_> = nodes
                .into_iter()
                .map(|node| {
                    scope.spawn(move || {
                        let host = if node.host.is_empty() {
                            node.addr.clone()
                        } else {
                            node.host.clone()
                        };
                        let result = Self::execute_remote(&node.addr, sql);
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

    pub fn execute_remote_query(addr: &str, sql: &str) -> Result<DataFrame> {
        Self::execute_remote(addr, sql)
    }

    fn execute_remote(addr: &str, sql: &str) -> Result<DataFrame> {
        #[cfg(any(test, feature = "test-utils"))]
        if let Some(hook) = lock_remote_query_hook().as_ref() {
            return hook(addr, sql);
        }

        let url = format!("http://{addr}/query");
        let request = Message::new(Query {
            expr: sql.to_string(),
            ..Default::default()
        });
        let body =
            serde_json::to_string(&request).map_err(|e| DataFusionError::External(Box::new(e)))?;
        let addr_owned = addr.to_string();
        let response = ureq::post(&url)
            .config()
            .timeout_global(Some(remote_query_timeout()))
            .build()
            .send(body)
            .map_err(|e| DataFusionError::External(Box::new(e)))?;

        let status = response.status().as_u16();
        let text = response
            .into_body()
            .read_to_string()
            .map_err(|e| DataFusionError::External(Box::new(e)))?;
        if status >= 400 {
            return Err(DataFusionError::Execution(format!(
                "remote query {addr_owned} failed: HTTP {status}: {text}"
            )));
        }

        let msg: Message<QueryDataFormat> =
            serde_json::from_str(&text).map_err(|e| DataFusionError::External(Box::new(e)))?;
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
