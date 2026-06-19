use super::ApiClient;
use crate::utils::error::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StepDurationSample {
    pub rank: i32,
    pub local_step: i64,
    pub duration_ms: f64,
    pub host: String,
    pub addr: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StepMatrixResponse {
    pub samples: Vec<StepDurationSample>,
    pub rank_count: usize,
    pub step_count: usize,
    pub cluster: bool,
    pub nodes_queried: usize,
    pub nodes_failed: Vec<String>,
}

impl ApiClient {
    /// Cross-rank ``train.step`` durations for straggler heatmaps.
    pub async fn fetch_step_matrix(
        &self,
        limit: usize,
        cluster: bool,
    ) -> Result<StepMatrixResponse> {
        let response = self
            .get_request(&format!(
                "/apis/training/step_matrix?limit={limit}&cluster={cluster}"
            ))
            .await?;
        Self::parse_json(&response)
    }
}
