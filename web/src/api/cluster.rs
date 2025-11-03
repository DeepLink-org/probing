use super::ApiClient;
use crate::utils::error::Result;
use probing_proto::prelude::*;

/// 集群管理API
impl ApiClient {
    /// 获取所有节点信息
    pub async fn get_nodes(&self) -> Result<Vec<Node>> {
        let response = self.get_request("/apis/nodes").await?;
        Self::parse_json(&response)
    }
}

