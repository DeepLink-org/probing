use probing_core::core::cluster::{apply_node_report, get_nodes as core_get_nodes};
use probing_proto::prelude::{Node, NodeReportRequest, NodeReportResponse};
use serde::Deserialize;

use super::error::{ApiError, ApiResult};

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum PutNodesBody {
    Batch(NodeReportRequest),
    Single(Node),
}

/// Register or heartbeat one or more cluster nodes; returns merged snapshot.
///
/// Cluster merge runs on the blocking pool so the shared Tokio runtime stays responsive.
pub(crate) async fn put_node(
    axum::Json(body): axum::Json<PutNodesBody>,
) -> ApiResult<axum::Json<NodeReportResponse>> {
    let nodes = match body {
        PutNodesBody::Single(node) => vec![node],
        PutNodesBody::Batch(req) => req.nodes,
    };
    let resp = tokio::task::spawn_blocking(move || apply_node_report(nodes))
        .await
        .map_err(|e| ApiError::internal(format!("cluster report task failed: {e}")))?;
    Ok(axum::Json(resp))
}

/// Get all nodes in the cluster as JSON
pub async fn get_nodes() -> ApiResult<axum::Json<Vec<Node>>> {
    Ok(axum::Json(core_get_nodes()))
}
