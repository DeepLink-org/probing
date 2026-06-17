use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use datafusion::arrow::datatypes::SchemaRef;
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::error::{DataFusionError, Result};
use probing_proto::prelude::{DataFrame, Message, Query, QueryDataFormat};

use crate::core::cluster::get_nodes;

use super::convert::{align_batch_to_schema, dataframe_to_record_batch};

const REMOTE_QUERY_TIMEOUT: Duration = Duration::from_secs(2);

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

    pub fn collect_remote_batches(
        sql: &str,
        output_schema: &SchemaRef,
    ) -> Result<Vec<RecordBatch>> {
        let mut stats = FanoutStats::default();
        let mut batches = Vec::new();
        let local_addrs = Self::local_listen_addrs();
        for node in get_nodes() {
            if local_addrs.iter().any(|local| local == &node.addr) {
                continue;
            }
            let host = if node.host.is_empty() {
                node.addr.clone()
            } else {
                node.host.clone()
            };
            match Self::execute_remote(&node.addr, sql) {
                Ok(df) => {
                    stats.nodes_succeeded += 1;
                    let batch = dataframe_to_record_batch(&df, &host, &node.addr, node.rank)?;
                    if batch.num_rows() > 0 {
                        batches.push(align_batch_to_schema(batch, output_schema.as_ref())?);
                    }
                }
                Err(err) => {
                    log::debug!("federated query skipped {}: {err}", node.addr);
                    stats.nodes_failed.push(node.addr.clone());
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
        let response = std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    ureq::post(&url)
                        .config()
                        .timeout_global(Some(REMOTE_QUERY_TIMEOUT))
                        .build()
                        .send(body)
                        .map_err(|e| DataFusionError::External(Box::new(e)))
                })
                .join()
                .map_err(|_| DataFusionError::Execution("remote query thread panicked".into()))?
        })?;

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
