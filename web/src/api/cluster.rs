use super::ApiClient;
use crate::utils::error::Result;
use probing_proto::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterQueryRequest {
    pub expr: String,
    pub cluster: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterQueryMeta {
    pub cluster: bool,
    pub nodes_queried: usize,
    pub nodes_failed: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterQueryResponse {
    pub dataframe: DataFrame,
    pub meta: ClusterQueryMeta,
}

/// Cluster management API
impl ApiClient {
    /// Get all node information (paginated server-side; fetches all pages).
    pub async fn get_nodes(&self) -> Result<Vec<Node>> {
        let mut all = Vec::new();
        let mut offset = 0usize;
        loop {
            let response = self
                .get_request(&format!("/apis/nodes?offset={offset}&limit=1024"))
                .await?;
            let page: NodeListResponse = Self::parse_json(&response)?;
            let empty = page.nodes.is_empty();
            all.extend(page.nodes);
            if all.len() >= page.total || empty {
                break;
            }
            offset = offset.saturating_add(1024);
        }
        Ok(all)
    }

    /// On-demand SQL fan-out across cluster nodes (`cluster=true`) or local only.
    pub async fn cluster_query(&self, expr: &str, cluster: bool) -> Result<ClusterQueryResponse> {
        let body = serde_json::to_string(&ClusterQueryRequest {
            expr: expr.to_string(),
            cluster,
        })
        .map_err(|e| crate::utils::error::AppError::Api(e.to_string()))?;
        let response = self
            .post_request_with_body("/apis/cluster/query", body)
            .await?;
        Self::parse_json(&response)
    }
}
