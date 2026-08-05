use async_trait::async_trait;
use datafusion::error::{DataFusionError, Result as DataFusionResult};
use probing_core::core::federation::{
    fanout_strict_enabled, remote_query_timeout, FanoutScope, FanoutStats, PeerQueryOutcome,
    PeerQueryTransport,
};
use probing_proto::prelude::*;

use super::types::FanoutOutcome;
use crate::auth::peer_auth_header_value;

#[async_trait]
pub(super) trait PeerQueryClient: Send + Sync {
    async fn query_leaf(&self, addr: &str, sql: &str) -> anyhow::Result<DataFrame>;
    async fn query_node(&self, addr: &str, sql: &str) -> anyhow::Result<FanoutOutcome>;
}

#[derive(Debug, Default, Clone, Copy)]
pub(super) struct HttpPeerQueryClient;

#[derive(Debug, Default)]
struct HttpFederationPeerTransport;

#[derive(Debug)]
struct PeerTransportError(anyhow::Error);

impl std::fmt::Display for PeerTransportError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{:#}", self.0)
    }
}

impl std::error::Error for PeerTransportError {}

fn transport_error(error: anyhow::Error) -> DataFusionError {
    DataFusionError::External(Box::new(PeerTransportError(error)))
}

impl PeerQueryTransport for HttpFederationPeerTransport {
    fn query(
        &self,
        addr: &str,
        sql: &str,
        scope: FanoutScope,
    ) -> DataFusionResult<PeerQueryOutcome> {
        if scope == FanoutScope::Coordinator {
            return remote_node_aggregate_blocking(addr, sql)
                .map(peer_query_outcome)
                .map_err(transport_error);
        }
        remote_query_df_blocking(addr, sql)
            .map(PeerQueryOutcome::complete)
            .map_err(transport_error)
    }
}

fn peer_query_outcome(outcome: FanoutOutcome) -> PeerQueryOutcome {
    let meta = outcome.meta;
    PeerQueryOutcome::with_stats(
        outcome.dataframe,
        FanoutStats {
            nodes_succeeded: meta.nodes_queried.saturating_sub(meta.nodes_failed.len()),
            nodes_failed: meta.nodes_failed,
            peer_batches_dropped: meta.peer_batches_dropped,
            partial: meta.partial,
        },
    )
}

pub(crate) fn core_transport() -> std::sync::Arc<dyn PeerQueryTransport> {
    std::sync::Arc::new(HttpFederationPeerTransport)
}

#[async_trait]
impl PeerQueryClient for HttpPeerQueryClient {
    async fn query_leaf(&self, addr: &str, sql: &str) -> anyhow::Result<DataFrame> {
        remote_query_df(addr, sql).await
    }

    async fn query_node(&self, addr: &str, sql: &str) -> anyhow::Result<FanoutOutcome> {
        remote_node_aggregate(addr, sql).await
    }
}

pub async fn remote_query_df(addr: &str, sql: &str) -> anyhow::Result<DataFrame> {
    let addr = addr.to_string();
    let sql = sql.to_string();
    tokio::task::spawn_blocking(move || remote_query_df_blocking(&addr, &sql)).await?
}

fn remote_query_df_blocking(addr: &str, sql: &str) -> anyhow::Result<DataFrame> {
    let url = format!("http://{addr}/query");
    let request = Message::new(Query {
        expr: sql.to_string(),
        ..Default::default()
    });
    let body = serde_json::to_string(&request)?;
    let response = send_post_blocking(url, None, body)?;
    parse_leaf_response(response.0, &response.1, addr)
}

async fn remote_node_aggregate(addr: &str, sql: &str) -> anyhow::Result<FanoutOutcome> {
    let addr = addr.to_string();
    let sql = sql.to_string();
    tokio::task::spawn_blocking(move || remote_node_aggregate_blocking(&addr, &sql)).await?
}

fn remote_node_aggregate_blocking(addr: &str, sql: &str) -> anyhow::Result<FanoutOutcome> {
    let url = format!("http://{addr}/apis/cluster/query");
    let body = serde_json::to_string(&serde_json::json!({
        "expr": sql,
        "cluster": true,
        "hierarchical": true,
        "scope": "node",
    }))?;
    let response = send_post_blocking(url, Some("application/json"), body)?;
    parse_node_response(response.0, &response.1, addr)
}

fn send_post_blocking(
    url: String,
    content_type: Option<&'static str>,
    body: String,
) -> anyhow::Result<(u16, String)> {
    let timeout = remote_query_timeout();
    let mut request = ureq::post(&url)
        .config()
        .timeout_global(Some(timeout))
        .build();
    if let Some(content_type) = content_type {
        request = request.header("Content-Type", content_type);
    }
    if let Some(value) = peer_auth_header_value() {
        request = request.header("Authorization", value);
    }
    let response = request.send(body).map_err(anyhow::Error::new)?;
    let status = response.status().as_u16();
    let text = response.into_body().read_to_string()?;
    Ok((status, text))
}

fn parse_leaf_response(status: u16, text: &str, addr: &str) -> anyhow::Result<DataFrame> {
    if status >= 400 {
        if status == 503 && !fanout_strict_enabled() {
            if let Ok(dataframe) = decode_query_message_dataframe(text) {
                log::warn!("accepted partial 503 dataframe from {addr}");
                return Ok(dataframe);
            }
            if let Ok(response) = decode_cluster_query_response(text) {
                log::warn!("accepted partial 503 cluster response from {addr}");
                return Ok(response.dataframe);
            }
        }
        anyhow::bail!("HTTP {status}: {text}");
    }
    decode_query_message_dataframe(text)
}

fn parse_node_response(status: u16, text: &str, addr: &str) -> anyhow::Result<FanoutOutcome> {
    if status >= 400 {
        if status == 503 && !fanout_strict_enabled() {
            if let Ok(response) = decode_cluster_query_response(text) {
                log::warn!("accepted partial 503 node aggregate from {addr}");
                return Ok(response);
            }
        }
        anyhow::bail!("HTTP {status}: {text}");
    }
    decode_cluster_query_response(text)
}

fn decode_query_message_dataframe(text: &str) -> anyhow::Result<DataFrame> {
    let message: Message<QueryDataFormat> = serde_json::from_str(text)?;
    match message.payload {
        QueryDataFormat::DataFrame(dataframe) => Ok(dataframe),
        QueryDataFormat::Nil => Ok(DataFrame::default()),
        QueryDataFormat::Error(error) => anyhow::bail!("remote query: {}", error.message),
        QueryDataFormat::TimeSeries(_) => anyhow::bail!("unexpected timeseries"),
    }
}

fn decode_cluster_query_response(text: &str) -> anyhow::Result<FanoutOutcome> {
    let value: serde_json::Value = serde_json::from_str(text)?;
    if let Some(error) = value.get("error").and_then(|value| value.as_str()) {
        anyhow::bail!("remote cluster query: {error}");
    }
    Ok(serde_json::from_value(value)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::cluster_fanout::FanoutMeta;

    #[test]
    fn core_outcome_preserves_child_partial_metadata() {
        let outcome = peer_query_outcome(FanoutOutcome {
            dataframe: DataFrame::default(),
            meta: FanoutMeta {
                cluster: true,
                hierarchical: true,
                scope: "node".into(),
                nodes_queried: 3,
                nodes_failed: vec!["rank-3: timeout".into()],
                peer_batches_dropped: 1,
                node_aggregators_queried: 0,
                local_ranks_queried: 2,
                partial: true,
            },
        });

        assert_eq!(outcome.stats.nodes_succeeded, 2);
        assert_eq!(outcome.stats.nodes_failed, vec!["rank-3: timeout"]);
        assert_eq!(outcome.stats.peer_batches_dropped, 1);
        assert!(outcome.stats.partial);
    }
}
