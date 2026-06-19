use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use datafusion::arrow::datatypes::SchemaRef;
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::error::{DataFusionError, Result};
use probing_proto::prelude::{DataFrame, Message, Node, Query, QueryDataFormat};

use crate::core::cluster::get_nodes;

use super::convert::{align_batch_to_schema, dataframe_to_record_batch};

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

pub fn reset_fanout_stats() {
    *LAST_FANOUT_STATS.lock().unwrap() = FanoutStats::default();
}

/// Record the fan-out outcome so callers (e.g. cluster fan-out meta) can report
/// how many peers were actually queried and which ones failed.
pub fn set_fanout_stats(stats: FanoutStats) {
    *LAST_FANOUT_STATS.lock().unwrap() = stats;
}

/// Increment the success counter for one peer (concurrency-safe).
///
/// Used by streaming fan-out where each peer partition reports its own outcome.
pub fn record_fanout_success() {
    LAST_FANOUT_STATS.lock().unwrap().nodes_succeeded += 1;
}

/// Record a failed peer (concurrency-safe).
pub fn record_fanout_failure(addr: &str) {
    LAST_FANOUT_STATS
        .lock()
        .unwrap()
        .nodes_failed
        .push(addr.to_string());
}

pub fn take_fanout_stats() -> FanoutStats {
    std::mem::take(&mut *LAST_FANOUT_STATS.lock().unwrap())
}

pub struct ProbeClusterExecutor;

impl ProbeClusterExecutor {
    pub fn local_host_label() -> String {
        std::env::var("HOSTNAME")
            .or_else(|_| std::env::var("HOST"))
            .unwrap_or_else(|_| "localhost".into())
    }

    pub fn local_listen_addrs() -> Vec<String> {
        std::env::var("PROBING_ADDRESS")
            .map(|addr| vec![addr])
            .unwrap_or_else(|_| vec!["127.0.0.1:8080".into()])
    }

    pub fn local_addr_label() -> String {
        Self::local_listen_addrs()
            .into_iter()
            .next()
            .unwrap_or_else(|| "127.0.0.1:8080".into())
    }

    /// Peer nodes that are not the local node (deduplicated against listen addrs).
    pub fn remote_nodes() -> Vec<Node> {
        let local_addrs = Self::local_listen_addrs();
        get_nodes()
            .into_iter()
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

    pub fn collect_remote_batches(
        sql: &str,
        output_schema: &SchemaRef,
    ) -> Result<Vec<RecordBatch>> {
        let mut stats = FanoutStats::default();
        let mut batches = Vec::new();
        for outcome in Self::fanout_query_to_peers(sql) {
            match outcome.result {
                Ok(df) => {
                    stats.nodes_succeeded += 1;
                    let batch =
                        dataframe_to_record_batch(&df, &outcome.host, &outcome.addr, outcome.rank)?;
                    if batch.num_rows() > 0 {
                        batches.push(align_batch_to_schema(batch, output_schema.as_ref())?);
                    }
                }
                Err(err) => {
                    log::debug!("federated query skipped {}: {err}", outcome.addr);
                    stats.nodes_failed.push(outcome.addr);
                }
            }
        }
        *LAST_FANOUT_STATS.lock().unwrap() = stats;
        Ok(batches)
    }

    pub fn execute_remote_query(addr: &str, sql: &str) -> Result<DataFrame> {
        Self::execute_remote(addr, sql)
    }

    fn execute_remote(addr: &str, sql: &str) -> Result<DataFrame> {
        let url = format!("http://{addr}/query");
        let request = Message::new(Query {
            expr: sql.to_string(),
            ..Default::default()
        });
        let body = serde_json::to_string(&request)
            .map_err(|e| DataFusionError::External(Box::new(e)))?;
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

        let msg: Message<QueryDataFormat> = serde_json::from_str(&text)
            .map_err(|e| DataFusionError::External(Box::new(e)))?;
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
