use super::ApiClient;
use crate::utils::error::Result;
use probing_proto::prelude::*;

/// 系统概览API
impl ApiClient {
    /// 获取系统概览信息
    pub async fn get_overview(&self) -> Result<Process> {
        let response = self.get_request("/apis/overview").await?;
        Self::parse_json(&response)
    }
}
