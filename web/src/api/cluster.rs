use super::ApiClient;
use crate::utils::error::Result;
use probing_proto::prelude::*;

/// Cluster management API
impl ApiClient {
    /// Get all node information
    pub async fn get_nodes(&self) -> Result<Vec<Node>> {
        let response = self.get_request("/apis/nodes").await?;
        Self::parse_json(&response)
    }
}

