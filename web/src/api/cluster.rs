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
    /// Get all node information
    pub async fn get_nodes(&self) -> Result<Vec<Node>> {
        let response = self.get_request("/apis/nodes").await?;
        Self::parse_json(&response)
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
