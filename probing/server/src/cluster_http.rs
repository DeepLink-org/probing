use std::time::Duration;

use anyhow::Result;
use probing_core::core::federation::{FanoutHttpMethod, FanoutHttpRequest};
use probing_proto::prelude::{Node, NodeListResponse, NodeReportRequest, NodeReportResponse};

fn peer_addr(http_base: &str) -> &str {
    http_base
        .trim_end_matches('/')
        .trim_start_matches("http://")
        .trim_start_matches("https://")
}

pub fn get_i32_env(name: &str) -> Option<i32> {
    std::env::var(name)
        .ok()
        .filter(|v| !v.is_empty())
        .and_then(|v| v.parse().ok())
}

fn nodes_page_size() -> usize {
    std::env::var("PROBING_CLUSTER_NODES_PAGE_SIZE")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or(1024)
}

pub fn fetch_nodes_blocking(http_base: &str) -> Result<Vec<Node>> {
    let addr = peer_addr(http_base);
    let page_size = nodes_page_size();
    let mut offset = 0usize;
    let mut all = Vec::new();
    loop {
        let response = crate::server::cluster_fanout::request_peer_blocking(
            addr,
            FanoutHttpRequest {
                method: FanoutHttpMethod::Get,
                path: format!("/apis/nodes?offset={offset}&limit={page_size}"),
                content_type: None,
                body: Vec::new(),
                timeout: Duration::from_secs(10),
            },
        )?;
        if response.status >= 400 {
            anyhow::bail!(
                "HTTP {}: {}",
                response.status,
                String::from_utf8_lossy(&response.body)
            );
        }
        let resp: NodeListResponse = serde_json::from_slice(&response.body)?;
        let empty = resp.nodes.is_empty();
        all.extend(resp.nodes);
        if all.len() >= resp.total || empty {
            break;
        }
        offset = offset.saturating_add(page_size);
    }
    Ok(all)
}

pub fn put_nodes_blocking(
    http_base: &str,
    nodes: Vec<Node>,
    seen_version: u64,
) -> Result<NodeReportResponse> {
    let addr = peer_addr(http_base);
    let response = crate::server::cluster_fanout::request_peer_blocking(
        addr,
        node_report_request(nodes, seen_version)?,
    )?;
    decode_node_report(response)
}

pub async fn put_nodes(
    http_base: &str,
    nodes: Vec<Node>,
    seen_version: u64,
) -> Result<NodeReportResponse> {
    let response = crate::server::cluster_fanout::core_service()?
        .request_peer(
            peer_addr(http_base),
            node_report_request(nodes, seen_version)?,
        )
        .await
        .map_err(anyhow::Error::new)?;
    decode_node_report(response)
}

fn node_report_request(nodes: Vec<Node>, seen_version: u64) -> Result<FanoutHttpRequest> {
    Ok(FanoutHttpRequest {
        method: FanoutHttpMethod::Put,
        path: "/apis/nodes".into(),
        content_type: Some("application/json".into()),
        body: serde_json::to_vec(&NodeReportRequest {
            nodes,
            seen_version,
        })?,
        timeout: Duration::from_secs(
            std::env::var("PROBING_CLUSTER_REPORT_TIMEOUT_SEC")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(5),
        ),
    })
}

fn decode_node_report(
    response: probing_core::core::federation::FanoutHttpResponse,
) -> Result<NodeReportResponse> {
    if response.status >= 400 {
        anyhow::bail!(
            "HTTP {}: {}",
            response.status,
            String::from_utf8_lossy(&response.body)
        );
    }
    Ok(serde_json::from_slice(&response.body)?)
}
