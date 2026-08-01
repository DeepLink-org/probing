use async_trait::async_trait;
use probing_core::core::federation::{fanout_strict_enabled, remote_query_timeout};
use probing_proto::prelude::*;

use super::types::FanoutOutcome;

#[async_trait]
pub(super) trait PeerQueryClient: Send + Sync {
    async fn query_leaf(&self, addr: &str, sql: &str) -> anyhow::Result<DataFrame>;
    async fn query_node(&self, addr: &str, sql: &str) -> anyhow::Result<FanoutOutcome>;
}

#[derive(Debug, Default, Clone, Copy)]
pub(super) struct HttpPeerQueryClient;

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
    let url = format!("http://{addr}/query");
    let request = Message::new(Query {
        expr: sql.to_string(),
        ..Default::default()
    });
    let body = serde_json::to_string(&request)?;
    let response = send_post(url, None, body).await?;
    parse_leaf_response(response.0, &response.1, addr)
}

async fn remote_node_aggregate(addr: &str, sql: &str) -> anyhow::Result<FanoutOutcome> {
    let url = format!("http://{addr}/apis/cluster/query");
    let body = serde_json::to_string(&serde_json::json!({
        "expr": sql,
        "cluster": true,
        "hierarchical": true,
        "scope": "node",
    }))?;
    let response = send_post(url, Some("application/json"), body).await?;
    parse_node_response(response.0, &response.1, addr)
}

async fn send_post(
    url: String,
    content_type: Option<&'static str>,
    body: String,
) -> anyhow::Result<(u16, String)> {
    let timeout = remote_query_timeout();
    tokio::task::spawn_blocking(move || {
        let mut request = ureq::post(&url)
            .config()
            .timeout_global(Some(timeout))
            .build();
        if let Some(content_type) = content_type {
            request = request.header("Content-Type", content_type);
        }
        let response = request.send(body).map_err(anyhow::Error::new)?;
        let status = response.status().as_u16();
        let text = response.into_body().read_to_string()?;
        Ok((status, text))
    })
    .await?
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
