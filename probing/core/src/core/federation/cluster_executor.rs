use std::fmt::Debug;
use std::sync::{Arc, LazyLock};
#[cfg(any(test, feature = "test-utils"))]
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use datafusion::error::{DataFusionError, Result};
use probing_proto::prelude::{DataFrame, Node};

use crate::core::cluster::{
    hierarchical_metadata_available, hierarchical_metadata_unavailable_err, local_leaf_peers,
    node_aggregator_peers, remote_peers_excluding_local,
};
use crate::core::federation::fanout_scope::{
    current_fanout_scope, current_fanout_stats_handle, resolve_fanout_scope, FanoutScope,
};

#[cfg(any(test, feature = "test-utils"))]
type RemoteQueryHook = Box<dyn Fn(&str, &str) -> Result<DataFrame> + Send + Sync>;

#[cfg(any(test, feature = "test-utils"))]
static REMOTE_QUERY_HOOK: LazyLock<Mutex<Option<RemoteQueryHook>>> =
    LazyLock::new(|| Mutex::new(None));

/// L3-provided transport for L1 federation execution.
pub trait PeerQueryTransport: Debug + Send + Sync {
    fn query(&self, addr: &str, sql: &str, scope: FanoutScope) -> Result<PeerQueryOutcome>;
}

/// Data and completeness metadata returned for one remote subtree.
///
/// `stats` accounts for the addressed endpoint and every descendant it queried.
/// A leaf success therefore reports one successful node, while a node aggregator
/// reports the complete subtree counts received from its L3 response.
#[derive(Debug)]
pub struct PeerQueryOutcome {
    pub dataframe: DataFrame,
    pub stats: FanoutStats,
}

impl PeerQueryOutcome {
    pub fn complete(dataframe: DataFrame) -> Self {
        Self {
            dataframe,
            stats: FanoutStats::complete_node(),
        }
    }

    pub fn with_stats(dataframe: DataFrame, stats: FanoutStats) -> Self {
        Self { dataframe, stats }
    }
}

/// Install an in-process remote query handler for federation integration tests.
#[cfg(any(test, feature = "test-utils"))]
pub fn set_remote_query_hook(hook: Option<RemoteQueryHook>) {
    *lock_remote_query_hook() = hook;
}

/// Default per-node timeout for remote federated queries (seconds).
const DEFAULT_REMOTE_QUERY_TIMEOUT_SECS: u64 = 30;
/// Env var to override the per-node remote query timeout (seconds).
const REMOTE_QUERY_TIMEOUT_ENV: &str = "PROBING_REMOTE_QUERY_TIMEOUT_SECS";
/// Max concurrent remote fan-out requests (HTTP or in-process federation).
const REMOTE_FANOUT_CONCURRENCY_ENV: &str = "PROBING_FANOUT_CONCURRENCY";
const DEFAULT_REMOTE_FANOUT_CONCURRENCY: usize = 128;

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

/// Max concurrent in-flight remote fan-out requests per query.
///
/// Defaults to [`DEFAULT_REMOTE_FANOUT_CONCURRENCY`]; override via
/// `PROBING_FANOUT_CONCURRENCY`. A value of `0` (or unparseable) falls back
/// to the default.
pub fn remote_fanout_concurrency() -> usize {
    std::env::var(REMOTE_FANOUT_CONCURRENCY_ENV)
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|&v| v > 0)
        .unwrap_or(DEFAULT_REMOTE_FANOUT_CONCURRENCY)
}

/// Outcome of a remote query against a single peer, retaining node identity so
/// callers can tag rows and account for successes/failures.
pub struct RemoteFanoutResult {
    pub addr: String,
    pub host: String,
    pub rank: Option<i32>,
    pub result: Result<PeerQueryOutcome>,
}

/// One remote partition in a raw federated scan.
///
/// Coordinator scans deliberately mix direct on-node leaf queries with
/// node-aggregate queries to remote nodes, so the routing scope belongs to
/// each target rather than to the scan as a whole.
#[derive(Debug, Clone)]
pub(crate) struct FederatedScanTarget {
    pub node: Node,
    pub scope: FanoutScope,
}

pub use super::fanout_scope::FanoutStats;

#[cfg(any(test, feature = "test-utils"))]
fn lock_remote_query_hook() -> MutexGuard<'static, Option<RemoteQueryHook>> {
    crate::sync::lock_mutex(&REMOTE_QUERY_HOOK, "REMOTE_QUERY_HOOK")
}

pub fn reset_fanout_stats() {
    current_fanout_stats_handle().reset();
}

/// Record the fan-out outcome so callers (e.g. cluster fan-out meta) can report
/// how many peers were actually queried and which ones failed.
pub fn set_fanout_stats(stats: FanoutStats) {
    current_fanout_stats_handle().set(stats);
}

/// Env var: when set to `1` or `true`, federated fan-out failures fail the query.
const FANOUT_STRICT_ENV: &str = "PROBING_FANOUT_STRICT";

fn parse_fanout_strict_env() -> bool {
    std::env::var(FANOUT_STRICT_ENV)
        .ok()
        .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"))
}

#[cfg(not(test))]
static FANOUT_STRICT: LazyLock<bool> = LazyLock::new(parse_fanout_strict_env);

/// Whether federated fan-out must succeed on every peer (fail-fast).
pub fn fanout_strict_enabled() -> bool {
    #[cfg(test)]
    {
        parse_fanout_strict_env()
    }
    #[cfg(not(test))]
    {
        *FANOUT_STRICT
    }
}

pub fn fanout_stats_partial(stats: &FanoutStats) -> bool {
    stats.partial || !stats.nodes_failed.is_empty() || stats.peer_batches_dropped > 0
}

/// Fail the query when strict fan-out is enabled and any peer was dropped.
pub fn enforce_fanout_strict(stats: &FanoutStats) -> Result<()> {
    if fanout_strict_enabled() && fanout_stats_partial(stats) {
        return Err(DataFusionError::Execution(format!(
            "federated fan-out strict mode: {} node(s) failed, {} peer batch(es) dropped",
            stats.nodes_failed.len(),
            stats.peer_batches_dropped
        )));
    }
    Ok(())
}

pub fn take_fanout_stats() -> FanoutStats {
    current_fanout_stats_handle().take()
}

/// Check request-scoped fan-out stats without consuming them.
pub fn check_fanout_strict() -> Result<()> {
    enforce_fanout_strict(&current_fanout_stats_handle().snapshot())
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
                } else if crate::core::federation::fanout_scope::hierarchical_fanout_enabled() {
                    log::warn!(
                        "hierarchical fan-out metadata missing; refusing flat peer fallback (Coordinator scope)"
                    );
                    Vec::new()
                } else {
                    remote_peers_excluding_local()
                }
            }
            FanoutScope::Node => {
                if hierarchical_metadata_available() {
                    local_leaf_peers()
                } else if crate::core::federation::fanout_scope::hierarchical_fanout_enabled() {
                    log::warn!(
                        "hierarchical fan-out metadata missing; refusing flat peer fallback (Node scope)"
                    );
                    Vec::new()
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
    pub fn fanout_query_to_peers(
        sql: &str,
        transport: Option<Arc<dyn PeerQueryTransport>>,
    ) -> Vec<RemoteFanoutResult> {
        Self::fanout_query_to_peers_scoped(sql, current_fanout_scope(), transport)
    }

    pub fn fanout_query_to_peers_scoped(
        sql: &str,
        scope: FanoutScope,
        transport: Option<Arc<dyn PeerQueryTransport>>,
    ) -> Vec<RemoteFanoutResult> {
        let nodes = Self::remote_nodes_for_scope(scope);
        if nodes.is_empty() {
            return Vec::new();
        }
        let scope = resolve_fanout_scope(scope);
        let concurrency = remote_fanout_concurrency();
        let mut results = Vec::with_capacity(nodes.len());
        for chunk in nodes.chunks(concurrency) {
            std::thread::scope(|s| {
                let handles: Vec<_> = chunk
                    .iter()
                    .map(|node| {
                        let node = node.clone();
                        let transport = transport.clone();
                        s.spawn(move || {
                            let host = if node.host.is_empty() {
                                node.addr.clone()
                            } else {
                                node.host.clone()
                            };
                            let result = Self::execute_remote_scoped(
                                transport.as_ref(),
                                &node.addr,
                                sql,
                                scope,
                            );
                            RemoteFanoutResult {
                                addr: node.addr,
                                host,
                                rank: node.rank,
                                result,
                            }
                        })
                    })
                    .collect();
                for handle in handles {
                    results.push(handle.join().unwrap_or_else(|_| RemoteFanoutResult {
                        addr: String::new(),
                        host: String::new(),
                        rank: None,
                        result: Err(DataFusionError::Execution(
                            "remote query thread panicked".into(),
                        )),
                    }));
                }
            });
        }
        results
    }

    /// Remote partitions for a federated table scan.
    ///
    /// A coordinator owns its own local partition, queries sibling ranks on
    /// the same node directly, and asks one aggregator on every other node to
    /// fan in there. Hierarchical scopes fail closed when metadata is partial;
    /// silently switching to a flat scan can duplicate or omit ranks.
    pub(crate) fn federated_scan_targets() -> Result<Vec<FederatedScanTarget>> {
        let resolved = resolve_fanout_scope(current_fanout_scope());
        match resolved {
            FanoutScope::Coordinator => {
                if !hierarchical_metadata_available() {
                    return Err(hierarchical_metadata_unavailable_err().into());
                }
                Ok(Self::hierarchical_scan_targets(
                    local_leaf_peers(),
                    node_aggregator_peers(),
                ))
            }
            FanoutScope::Node => {
                if !hierarchical_metadata_available() {
                    return Err(hierarchical_metadata_unavailable_err().into());
                }
                Ok(local_leaf_peers()
                    .into_iter()
                    .map(|node| FederatedScanTarget {
                        node,
                        scope: FanoutScope::Node,
                    })
                    .collect())
            }
            scope => Ok(Self::remote_nodes_for_scope(scope)
                .into_iter()
                .map(|node| FederatedScanTarget { node, scope })
                .collect()),
        }
    }

    fn hierarchical_scan_targets(
        local_leaves: Vec<Node>,
        remote_aggregators: Vec<Node>,
    ) -> Vec<FederatedScanTarget> {
        local_leaves
            .into_iter()
            .map(|node| FederatedScanTarget {
                node,
                scope: FanoutScope::Node,
            })
            .chain(
                remote_aggregators
                    .into_iter()
                    .map(|node| FederatedScanTarget {
                        node,
                        scope: FanoutScope::Coordinator,
                    }),
            )
            .collect()
    }

    pub fn execute_remote_query(
        transport: Option<&Arc<dyn PeerQueryTransport>>,
        addr: &str,
        sql: &str,
    ) -> Result<PeerQueryOutcome> {
        Self::execute_remote_for_scope(transport, addr, sql, current_fanout_scope())
    }

    pub fn execute_remote_for_scope(
        transport: Option<&Arc<dyn PeerQueryTransport>>,
        addr: &str,
        sql: &str,
        scope: FanoutScope,
    ) -> Result<PeerQueryOutcome> {
        Self::execute_remote_scoped(transport, addr, sql, scope)
    }

    fn execute_remote_scoped(
        transport: Option<&Arc<dyn PeerQueryTransport>>,
        addr: &str,
        sql: &str,
        scope: FanoutScope,
    ) -> Result<PeerQueryOutcome> {
        let scope = resolve_fanout_scope(scope);
        #[cfg(any(test, feature = "test-utils"))]
        if let Some(hook) = lock_remote_query_hook().as_ref() {
            return hook(addr, sql).map(PeerQueryOutcome::complete);
        }
        let transport = transport.ok_or_else(|| {
            DataFusionError::Execution("peer query transport is not configured".to_string())
        })?;
        transport.query(addr, sql, scope)
    }
}

#[cfg(test)]
mod fanout_strict_tests {
    use super::*;

    #[test]
    fn enforce_fanout_strict_respects_env() {
        let stats = FanoutStats {
            nodes_failed: vec!["10.0.0.2:8080".into()],
            ..FanoutStats::default()
        };
        std::env::remove_var(FANOUT_STRICT_ENV);
        assert!(enforce_fanout_strict(&stats).is_ok());
        std::env::set_var(FANOUT_STRICT_ENV, "1");
        assert!(enforce_fanout_strict(&stats).is_err());
        std::env::remove_var(FANOUT_STRICT_ENV);
    }

    #[test]
    fn coordinator_raw_scan_includes_local_leaves_and_remote_aggregators() {
        let leaf = Node {
            addr: "10.0.0.1:8081".into(),
            rank: Some(1),
            ..Default::default()
        };
        let aggregator = Node {
            addr: "10.0.0.2:8080".into(),
            rank: Some(8),
            ..Default::default()
        };

        let targets = ProbeClusterExecutor::hierarchical_scan_targets(
            vec![leaf.clone()],
            vec![aggregator.clone()],
        );
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].node.addr, leaf.addr);
        assert_eq!(targets[0].scope, FanoutScope::Node);
        assert_eq!(targets[1].node.addr, aggregator.addr);
        assert_eq!(targets[1].scope, FanoutScope::Coordinator);
    }
}
