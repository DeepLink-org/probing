use probing_core::core::cluster::{apply_node_report, cluster_version, get_nodes_page};
use probing_proto::prelude::{Node, NodeListResponse, NodeReportRequest, NodeReportResponse};
use serde::Deserialize;

use super::error::{ApiError, ApiResult};

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum PutNodesBody {
    Batch(NodeReportRequest),
    Single(Node),
}

#[derive(Debug, Deserialize)]
pub(crate) struct GetNodesQuery {
    #[serde(default)]
    offset: usize,
    #[serde(default = "default_page_limit")]
    limit: usize,
    #[serde(default)]
    since_version: Option<u64>,
}

fn default_page_limit() -> usize {
    std::env::var("PROBING_CLUSTER_NODES_PAGE_SIZE")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or(1024)
}

/// Register or heartbeat one or more cluster nodes; returns merged snapshot.
///
/// Cluster merge runs on the blocking pool so the shared Tokio runtime stays responsive.
pub(crate) async fn put_node(
    axum::Json(body): axum::Json<PutNodesBody>,
) -> ApiResult<axum::Json<NodeReportResponse>> {
    let (nodes, seen_version) = match body {
        PutNodesBody::Single(node) => (vec![node], cluster_version()),
        PutNodesBody::Batch(req) => (req.nodes, req.seen_version),
    };
    let resp = tokio::task::spawn_blocking(move || apply_node_report(nodes, seen_version))
        .await
        .map_err(|e| ApiError::internal(format!("cluster report task failed: {e}")))?;
    Ok(axum::Json(resp))
}

/// Paginated cluster node list (`offset`, `limit`, optional `since_version`).
pub(crate) async fn get_nodes(
    axum::extract::Query(query): axum::extract::Query<GetNodesQuery>,
) -> ApiResult<axum::Json<NodeListResponse>> {
    let limit = query.limit.min(10_000);
    let (version, total, nodes) = tokio::task::spawn_blocking(move || {
        get_nodes_page(query.offset, limit, query.since_version)
    })
    .await
    .map_err(|e| ApiError::internal(format!("cluster list task failed: {e}")))?;
    Ok(axum::Json(NodeListResponse {
        version,
        total,
        offset: query.offset,
        nodes,
    }))
}
