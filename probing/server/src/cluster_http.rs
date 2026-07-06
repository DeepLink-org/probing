use std::time::Duration;

use anyhow::Result;
use probing_proto::prelude::{Node, NodeListResponse, NodeReportRequest, NodeReportResponse};

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
    let base = http_base.trim_end_matches('/');
    let page_size = nodes_page_size();
    let mut offset = 0usize;
    let mut all = Vec::new();
    loop {
        let url = format!("{base}/apis/nodes?offset={offset}&limit={page_size}");
        let text = ureq::get(&url)
            .config()
            .no_delay(true)
            .timeout_global(Some(Duration::from_secs(10)))
            .build()
            .call()?
            .body_mut()
            .read_to_string()?;
        let resp: NodeListResponse = serde_json::from_str(&text)?;
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
    let url = format!("{}/apis/nodes", http_base.trim_end_matches('/'));
    let body = NodeReportRequest {
        nodes,
        seen_version,
    };
    let text = ureq::put(&url)
        .config()
        .no_delay(true)
        .timeout_global(Some(Duration::from_secs(
            std::env::var("PROBING_CLUSTER_REPORT_TIMEOUT_SEC")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(5),
        )))
        .build()
        .send_json(body)?
        .body_mut()
        .read_to_string()?;
    Ok(serde_json::from_str(&text)?)
}
