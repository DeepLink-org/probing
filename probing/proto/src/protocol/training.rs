//! Stable wire types for training observability endpoints.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct StepDurationSample {
    pub rank: i32,
    /// Display index (chronological, zero-based window into recent steps).
    pub local_step: i64,
    /// Original `local_step` from span attributes.
    #[serde(default)]
    pub coord_step: i64,
    pub duration_ms: f64,
    pub host: String,
    pub addr: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct StepMatrixResponse {
    pub samples: Vec<StepDurationSample>,
    pub rank_count: usize,
    pub step_count: usize,
    pub cluster: bool,
    /// Some peers failed, so the samples are useful but incomplete.
    #[serde(default)]
    pub partial: bool,
    pub nodes_queried: usize,
    #[serde(default)]
    pub nodes_failed: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn older_step_matrix_without_partial_remains_readable() {
        let response: StepMatrixResponse = serde_json::from_str(
            r#"{"samples":[],"rank_count":0,"step_count":0,"cluster":false,"nodes_queried":1,"nodes_failed":[]}"#,
        )
        .unwrap();
        assert!(!response.partial);
    }
}
