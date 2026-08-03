use super::ApiClient;
use crate::utils::error::Result;

pub use probing_proto::protocol::training::{StepDurationSample, StepMatrixResponse};

impl ApiClient {
    /// Cross-rank ``train.step`` durations for straggler heatmaps.
    pub async fn fetch_step_matrix(
        &self,
        limit: usize,
        cluster: bool,
    ) -> Result<StepMatrixResponse> {
        let response = self
            .get_request_accepting(
                &format!("/apis/training/step_matrix?limit={limit}&cluster={cluster}"),
                &[503],
            )
            .await?;
        Self::parse_json(&response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_step_matrix_is_preserved_as_data() {
        let response: StepMatrixResponse = ApiClient::parse_json(
            r#"{"samples":[],"rank_count":0,"step_count":0,"cluster":true,"partial":true,"nodes_queried":8,"nodes_failed":["node-7"]}"#,
        )
        .unwrap();
        assert!(response.partial);
        assert_eq!(response.nodes_queried, 8);
        assert_eq!(response.nodes_failed, vec!["node-7"]);
    }
}
