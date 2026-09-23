//! Stable wire types for training observability endpoints.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default)]
pub struct RlRunSummary {
    pub run_id: String,
    pub framework: String,
    pub phase: String,
    pub global_step: i64,
    pub timestamp_ns: i64,
    pub start_time_ns: i64,
    pub end_time_ns: i64,
    pub samples_total: i64,
    pub tokens_total: i64,
    #[serde(default)]
    pub job_id: String,
    #[serde(default)]
    pub config_hash: String,
    #[serde(default)]
    pub metadata_json: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default)]
pub struct RlRunsResponse {
    pub runs: Vec<RlRunSummary>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct RlStatusResponse {
    pub run: RlRunSummary,
    #[serde(default)]
    pub sampler: Option<RlSamplerSnapshot>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct RlSamplerSnapshot {
    pub timestamp_ns: i64,
    pub step: i64,
    pub target: i64,
    pub accepted: i64,
    pub judged: i64,
    pub trained: i64,
    pub filtered: i64,
    pub failed: i64,
    pub expired: i64,
    pub in_flight: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default)]
pub struct RlTagsResponse {
    pub run_id: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default)]
pub struct RlMetricPoint {
    pub step: i64,
    pub wall_time_s: f64,
    pub timestamp_ns: i64,
    pub value: f64,
    /// Lowest raw value behind this point. `None` when the point is a raw
    /// reading rather than a summary of several.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub low: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub high: Option<f64>,
    /// Standard deviation of the raw values behind this point.
    ///
    /// Unlike `low`/`high`, this does not grow just because the point summarises
    /// more readings, so it is the honest thing to draw as a spread around the
    /// line: extremes widen without bound as buckets get coarser, until the band
    /// covers the whole plot and says nothing. `None` when a single reading.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spread: Option<f64>,
    /// Raw readings this point stands for. `0` from servers that do not
    /// aggregate, which callers should read as one.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub samples: i64,
}

fn is_zero(value: &i64) -> bool {
    *value == 0
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default)]
pub struct RlSeriesResponse {
    pub run_id: String,
    pub series: BTreeMap<String, Vec<RlMetricPoint>>,
    /// Width in trainer steps of each aggregated point, or `0` when the points
    /// are raw. Lets a client label a chart as summarised rather than exact.
    #[serde(default)]
    pub bucket_steps: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct RlSampleSummary {
    pub timestamp_ns: i64,
    pub step: i64,
    pub rollout_id: String,
    pub group_id: String,
    pub sample_id: String,
    pub task: String,
    pub status: String,
    pub reward: Option<f64>,
    pub reward_pass: bool,
    pub filter_reason: String,
    pub drop_reason: String,
    pub prompt_tokens: i64,
    pub response_tokens: i64,
    pub staleness: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default)]
pub struct RlSamplesResponse {
    pub run_id: String,
    pub samples: Vec<RlSampleSummary>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default)]
pub struct RlSamplerHistoryResponse {
    pub run_id: String,
    pub snapshots: Vec<RlSamplerSnapshot>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct RlCompositionBucket {
    pub dimension: String,
    pub key: String,
    pub count: i64,
    pub share: f64,
}

/// One trainer step's batch composition, oldest step first.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default)]
pub struct RlCompositionStep {
    pub step: i64,
    pub sample_count: i64,
    pub buckets: Vec<RlCompositionBucket>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default)]
pub struct RlCompositionResponse {
    pub run_id: String,
    pub sample_count: i64,
    pub buckets: Vec<RlCompositionBucket>,
    /// Per-step breakdown. Absent in responses from older servers.
    #[serde(default)]
    pub steps: Vec<RlCompositionStep>,
}

/// One slice of the per-prompt pass-rate distribution.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default)]
pub struct RlPassBucket {
    /// Position in the histogram, `0` for all-fail through `8` for all-pass.
    pub index: i64,
    pub label: String,
    pub prompts: i64,
    pub share: f64,
}

/// How pass rates are spread across prompts, which separates "every prompt is
/// middling" from "half are trivial and half are impossible" — two situations
/// that produce the same mean pass rate but call for different responses.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default)]
pub struct RlPassHistogramResponse {
    pub run_id: String,
    pub prompt_count: i64,
    pub rollout_count: i64,
    /// Mean of the per-prompt pass rates, weighting every prompt equally.
    pub avg_pass_rate: f64,
    pub buckets: Vec<RlPassBucket>,
}

/// Sampling outcome for one data source, so a dataset stuck in judging or
/// failing its filters can be spotted without querying `rl.sample` directly.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default)]
pub struct RlDatasetRow {
    pub task: String,
    pub category: String,
    pub samples: i64,
    /// Samples that finished the pipeline.
    pub completed: i64,
    pub filtered: i64,
    pub failed: i64,
    /// Samples still moving through the pipeline.
    pub in_flight: i64,
    /// Samples whose judge marked them as passing.
    pub passed: i64,
    /// `passed` over the judged samples, or `None` when nothing was judged.
    pub pass_rate: Option<f64>,
    /// Newest step this data source appeared in.
    pub last_step: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default)]
pub struct RlDatasetsResponse {
    pub run_id: String,
    pub sample_count: i64,
    /// Newest step covered by the scanned window.
    pub latest_step: i64,
    pub datasets: Vec<RlDatasetRow>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct RlStalenessBucket {
    pub label: String,
    pub min_staleness: i64,
    pub max_staleness: Option<i64>,
    pub count: i64,
    pub share: f64,
    pub tokens: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default)]
pub struct RlStalenessResponse {
    pub run_id: String,
    pub sample_count: i64,
    pub avg_staleness: f64,
    pub buckets: Vec<RlStalenessBucket>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct RlBenchmarkPoint {
    pub name: String,
    pub step: i64,
    pub score: f64,
    pub sample_count: i64,
    pub version: String,
    pub timestamp_ns: i64,
    /// Evaluation harness. Absent in responses from older servers.
    #[serde(default)]
    pub harness: String,
    /// How repeats were combined, such as `avg@3`.
    #[serde(default)]
    pub aggregation: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default)]
pub struct RlBenchmarksResponse {
    pub run_id: String,
    pub points: Vec<RlBenchmarkPoint>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default)]
pub struct RlAboutResponse {
    pub run: Option<RlRunSummary>,
    #[serde(default)]
    pub about_metrics: BTreeMap<String, f64>,
    #[serde(default)]
    pub metadata: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct RlEvent {
    pub timestamp_ns: i64,
    pub step: i64,
    pub level: String,
    pub kind: String,
    pub message: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default)]
pub struct RlEventsResponse {
    pub run_id: String,
    pub events: Vec<RlEvent>,
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
