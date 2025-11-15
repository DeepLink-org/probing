use super::ApiClient;
use crate::utils::error::Result;
use probing_proto::prelude::*;

/// System overview API
impl ApiClient {
    /// Get system overview information
    pub async fn get_overview(&self) -> Result<Process> {
        let response = self.get_request("/apis/overview").await?;
        Self::parse_json(&response)
    }
}
