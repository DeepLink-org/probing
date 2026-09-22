//! Stable read-only APIs for framework-neutral RL telemetry tables.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use axum::response::IntoResponse;
use axum::Json;
use probing_proto::prelude::{
    DataFrame, Ele, RlAboutResponse, RlBenchmarkPoint, RlBenchmarksResponse, RlCompositionBucket,
    RlCompositionResponse, RlCompositionStep, RlDatasetRow, RlDatasetsResponse, RlEvent,
    RlEventsResponse, RlMetricPoint, RlPassBucket, RlPassHistogramResponse, RlRunSummary,
    RlRunsResponse, RlSampleSummary, RlSamplerHistoryResponse, RlSamplerSnapshot,
    RlSamplesResponse, RlSeriesResponse, RlStalenessBucket, RlStalenessResponse, RlStatusResponse,
    RlTagsResponse,
};
use serde::Deserialize;

use super::cluster_fanout::{self, ClusterFanoutScope};
use super::error::{ApiError, ApiResult};

const MAX_RUN_ROWS: usize = 1_000;
const MAX_TAGS: usize = 5_000;
const MAX_SERIES_NAMES: usize = 96;
const DEFAULT_SERIES_LIMIT: usize = 2_000;
const MAX_SERIES_LIMIT: usize = 20_000;
const DEFAULT_SAMPLE_LIMIT: usize = 200;
const MAX_SAMPLE_LIMIT: usize = 2_000;
const DEFAULT_SAMPLER_LIMIT: usize = 40;
const MAX_SAMPLER_LIMIT: usize = 500;
const DEFAULT_COMPOSITION_LIMIT: usize = 2_000;
const MAX_COMPOSITION_LIMIT: usize = 20_000;
const MAX_COMPOSITION_BUCKETS: usize = 32;
const DEFAULT_STALENESS_LIMIT: usize = 2_000;
const MAX_STALENESS_LIMIT: usize = 20_000;
const DEFAULT_BENCHMARK_LIMIT: usize = 2_000;
const MAX_BENCHMARK_LIMIT: usize = 20_000;
const DEFAULT_EVENTS_LIMIT: usize = 40;
const MAX_EVENTS_LIMIT: usize = 200;

#[derive(Debug, Deserialize)]
pub struct RunQuery {
    pub run_id: String,
}

#[derive(Debug, Deserialize)]
pub struct SeriesQuery {
    pub run_id: String,
    pub names: String,
    pub limit: Option<usize>,
    /// When set, the server summarises each metric into at most this many points
    /// instead of returning raw readings under a shared row budget.
    pub buckets: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct SamplesQuery {
    pub run_id: String,
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct SamplerQuery {
    pub run_id: String,
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct CompositionQuery {
    pub run_id: String,
    pub limit: Option<usize>,
    pub dimension: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct StalenessQuery {
    pub run_id: String,
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct DatasetsQuery {
    pub run_id: String,
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct PassHistogramQuery {
    pub run_id: String,
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct BenchmarksQuery {
    pub run_id: String,
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct EventsQuery {
    pub run_id: String,
    pub limit: Option<usize>,
}

pub async fn get_runs() -> impl IntoResponse {
    runs().await.map(Json).map_err(IntoResponse::into_response)
}

pub async fn get_status(
    axum::extract::Query(params): axum::extract::Query<RunQuery>,
) -> impl IntoResponse {
    status(&params.run_id)
        .await
        .map(Json)
        .map_err(IntoResponse::into_response)
}

pub async fn get_tags(
    axum::extract::Query(params): axum::extract::Query<RunQuery>,
) -> impl IntoResponse {
    tags(&params.run_id)
        .await
        .map(Json)
        .map_err(IntoResponse::into_response)
}

pub async fn get_series(
    axum::extract::Query(params): axum::extract::Query<SeriesQuery>,
) -> impl IntoResponse {
    series(&params.run_id, &params.names, params.limit, params.buckets)
        .await
        .map(Json)
        .map_err(IntoResponse::into_response)
}

pub async fn get_samples(
    axum::extract::Query(params): axum::extract::Query<SamplesQuery>,
) -> impl IntoResponse {
    samples(&params.run_id, params.limit)
        .await
        .map(Json)
        .map_err(IntoResponse::into_response)
}

pub async fn get_sampler(
    axum::extract::Query(params): axum::extract::Query<SamplerQuery>,
) -> impl IntoResponse {
    sampler_history(&params.run_id, params.limit)
        .await
        .map(Json)
        .map_err(IntoResponse::into_response)
}

pub async fn get_composition(
    axum::extract::Query(params): axum::extract::Query<CompositionQuery>,
) -> impl IntoResponse {
    composition(&params.run_id, params.limit, params.dimension.as_deref())
        .await
        .map(Json)
        .map_err(IntoResponse::into_response)
}

pub async fn get_datasets(
    axum::extract::Query(params): axum::extract::Query<DatasetsQuery>,
) -> impl IntoResponse {
    datasets(&params.run_id, params.limit)
        .await
        .map(Json)
        .map_err(IntoResponse::into_response)
}

pub async fn get_pass_histogram(
    axum::extract::Query(params): axum::extract::Query<PassHistogramQuery>,
) -> impl IntoResponse {
    pass_histogram(&params.run_id, params.limit)
        .await
        .map(Json)
        .map_err(IntoResponse::into_response)
}

pub async fn get_staleness(
    axum::extract::Query(params): axum::extract::Query<StalenessQuery>,
) -> impl IntoResponse {
    staleness(&params.run_id, params.limit)
        .await
        .map(Json)
        .map_err(IntoResponse::into_response)
}

pub async fn get_benchmarks(
    axum::extract::Query(params): axum::extract::Query<BenchmarksQuery>,
) -> impl IntoResponse {
    benchmarks(&params.run_id, params.limit)
        .await
        .map(Json)
        .map_err(IntoResponse::into_response)
}

pub async fn get_events(
    axum::extract::Query(params): axum::extract::Query<EventsQuery>,
) -> impl IntoResponse {
    events(&params.run_id, params.limit)
        .await
        .map(Json)
        .map_err(IntoResponse::into_response)
}

pub async fn get_about(
    axum::extract::Query(params): axum::extract::Query<RunQuery>,
) -> impl IntoResponse {
    about(&params.run_id)
        .await
        .map(Json)
        .map_err(IntoResponse::into_response)
}

async fn runs() -> ApiResult<RlRunsResponse> {
    let sql = format!(
        "SELECT run_id, framework, phase, global_step, timestamp_ns, start_time_ns, \
         end_time_ns, samples_total, tokens_total, job_id, config_hash, metadata_json \
         FROM rl.run ORDER BY timestamp_ns DESC LIMIT {MAX_RUN_ROWS}"
    );
    let dataframe = query_local(&sql).await?;
    let mut seen = HashSet::new();
    let runs = (0..dataframe.row_count())
        .filter_map(|row| parse_run(&dataframe, row))
        .filter(|run| seen.insert(run.run_id.clone()))
        .collect();
    Ok(RlRunsResponse { runs })
}

async fn status(run_id: &str) -> ApiResult<RlStatusResponse> {
    require_value("run_id", run_id)?;
    let run_id = sql_literal(run_id);
    let sql = format!(
        "SELECT run_id, framework, phase, global_step, timestamp_ns, start_time_ns, \
         end_time_ns, samples_total, tokens_total, job_id, config_hash, metadata_json \
         FROM rl.run WHERE run_id = {run_id} ORDER BY timestamp_ns DESC LIMIT 1"
    );
    let dataframe = query_local(&sql).await?;
    let run =
        parse_run(&dataframe, 0).ok_or_else(|| ApiError::not_found("RL run was not found"))?;
    let sampler_sql = format!(
        "SELECT timestamp_ns, step, target, accepted, judged, trained, filtered, failed, \
         expired, in_flight FROM rl.sampler WHERE run_id = {run_id} \
         ORDER BY timestamp_ns DESC LIMIT 1"
    );
    let sampler_frame = query_local(&sampler_sql).await?;
    let sampler = parse_sampler(&sampler_frame, 0);
    Ok(RlStatusResponse { run, sampler })
}

async fn tags(run_id: &str) -> ApiResult<RlTagsResponse> {
    require_value("run_id", run_id)?;
    let run_literal = sql_literal(run_id);
    let sql = format!(
        "SELECT DISTINCT name FROM rl.metric WHERE run_id = {run_literal} \
         ORDER BY name LIMIT {MAX_TAGS}"
    );
    let dataframe = query_local(&sql).await?;
    let tags = (0..dataframe.row_count())
        .filter_map(|row| text_at(&dataframe, "name", row))
        .collect();
    Ok(RlTagsResponse {
        run_id: run_id.to_string(),
        tags,
    })
}

/// Series summarised into at most `buckets` points per metric.
///
/// The raw path shares one row budget across every metric requested, so asking
/// for eighteen charts over a long run truncates each of them to the most recent
/// steps and silently hides the earlier history. Aggregating in SQL instead makes
/// the response size independent of run length, so the full run stays visible.
async fn bucketed_series(
    run_id: &str,
    name_literals: &str,
    buckets: usize,
) -> ApiResult<RlSeriesResponse> {
    let buckets = buckets.clamp(1, MAX_SERIES_BUCKETS);
    let bounds_sql = format!(
        "SELECT min(step) AS lo, max(step) AS hi FROM rl.metric \
         WHERE run_id = {} AND name IN ({name_literals}) AND rank IN (-1, 0)",
        sql_literal(run_id)
    );
    let bounds = query_local(&bounds_sql).await?;
    let lo = bounds.scalar_i64("lo", 0);
    let hi = bounds.scalar_i64("hi", 0);
    // No rows at all, so there is nothing to bucket.
    let (Some(lo), Some(hi)) = (lo, hi) else {
        return Ok(RlSeriesResponse {
            run_id: run_id.to_string(),
            series: BTreeMap::new(),
            bucket_steps: 0,
        });
    };
    let width = bucket_width(lo, hi, buckets);
    let bucket_expr = format!("CAST((step - {lo}) / {width} AS BIGINT)");
    let sql = format!(
        "SELECT name, {bucket_expr} AS bucket, avg(value) AS avg_value, \
         min(value) AS min_value, max(value) AS max_value, \
         stddev(value) AS spread_value, count(value) AS samples, \
         max(step) AS last_step, max(timestamp_ns) AS last_ts, \
         max(wall_time_s) AS last_wall FROM rl.metric \
         WHERE run_id = {} AND name IN ({name_literals}) AND rank IN (-1, 0) \
         GROUP BY name, {bucket_expr} ORDER BY name ASC, bucket ASC",
        sql_literal(run_id)
    );
    let dataframe = query_local(&sql).await?;
    Ok(build_bucketed_series(run_id, &dataframe, width))
}

fn build_bucketed_series(run_id: &str, dataframe: &DataFrame, width: i64) -> RlSeriesResponse {
    let mut series: BTreeMap<String, Vec<RlMetricPoint>> = BTreeMap::new();
    for row in 0..dataframe.row_count() {
        let Some(name) = text_at(dataframe, "name", row) else {
            continue;
        };
        let Some(value) = dataframe.scalar_f64("avg_value", row) else {
            continue;
        };
        let low = dataframe.scalar_f64("min_value", row);
        let high = dataframe.scalar_f64("max_value", row);
        let samples = dataframe.scalar_i64("samples", row).unwrap_or(1).max(1);
        series.entry(name).or_default().push(RlMetricPoint {
            // The bucket is placed at its newest step, so the last point of a
            // chart sits on the newest telemetry rather than a bucket midpoint.
            step: dataframe.scalar_i64("last_step", row).unwrap_or(-1),
            wall_time_s: dataframe.scalar_f64("last_wall", row).unwrap_or(-1.0),
            timestamp_ns: dataframe.scalar_i64("last_ts", row).unwrap_or(0),
            value,
            low,
            high,
            // NULL for a single-reading bucket, where a sample stddev is undefined.
            spread: dataframe.scalar_f64("spread_value", row),
            samples,
        });
    }
    RlSeriesResponse {
        run_id: run_id.to_string(),
        series,
        bucket_steps: width,
    }
}

/// Most aggregated points a client may ask for per metric. A chart is a few
/// hundred pixels wide, so more than this cannot be drawn apart anyway.
const MAX_SERIES_BUCKETS: usize = 1_000;

/// Width in steps of each bucket so that `[lo, hi]` fits in `buckets` of them.
fn bucket_width(lo: i64, hi: i64, buckets: usize) -> i64 {
    let span = hi.saturating_sub(lo).saturating_add(1).max(1);
    let buckets = buckets.max(1) as i64;
    ((span + buckets - 1) / buckets).max(1)
}

async fn series(
    run_id: &str,
    names: &str,
    limit: Option<usize>,
    buckets: Option<usize>,
) -> ApiResult<RlSeriesResponse> {
    require_value("run_id", run_id)?;
    let names = names
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .take(MAX_SERIES_NAMES + 1)
        .collect::<Vec<_>>();
    if names.is_empty() {
        return Err(ApiError::bad_request(
            "names must contain at least one metric",
        ));
    }
    if names.len() > MAX_SERIES_NAMES {
        return Err(ApiError::bad_request(format!(
            "at most {MAX_SERIES_NAMES} metric names are allowed"
        )));
    }
    let name_literals = names
        .iter()
        .map(|name| sql_literal(name))
        .collect::<Vec<_>>()
        .join(", ");
    if let Some(buckets) = buckets {
        return bucketed_series(run_id, &name_literals, buckets).await;
    }
    let row_limit = limit
        .unwrap_or(DEFAULT_SERIES_LIMIT)
        .clamp(1, MAX_SERIES_LIMIT);
    let sql = format!(
        "SELECT name, step, wall_time_s, timestamp_ns, value FROM ( \
         SELECT name, step, wall_time_s, timestamp_ns, value FROM rl.metric \
         WHERE run_id = {} AND name IN ({name_literals}) AND rank IN (-1, 0) \
         ORDER BY timestamp_ns DESC LIMIT {row_limit} \
         ) recent ORDER BY step ASC, timestamp_ns ASC",
        sql_literal(run_id)
    );
    let dataframe = query_local(&sql).await?;
    let mut output: BTreeMap<String, Vec<RlMetricPoint>> = BTreeMap::new();
    for row in 0..dataframe.row_count() {
        let Some(name) = text_at(&dataframe, "name", row) else {
            continue;
        };
        let Some(value) = dataframe.scalar_f64("value", row) else {
            continue;
        };
        output.entry(name).or_default().push(RlMetricPoint {
            step: dataframe.scalar_i64("step", row).unwrap_or(-1),
            wall_time_s: dataframe.scalar_f64("wall_time_s", row).unwrap_or(-1.0),
            timestamp_ns: dataframe.scalar_i64("timestamp_ns", row).unwrap_or(0),
            value,
            ..Default::default()
        });
    }
    Ok(RlSeriesResponse {
        run_id: run_id.to_string(),
        series: output,
        bucket_steps: 0,
    })
}

async fn samples(run_id: &str, limit: Option<usize>) -> ApiResult<RlSamplesResponse> {
    require_value("run_id", run_id)?;
    let row_limit = limit
        .unwrap_or(DEFAULT_SAMPLE_LIMIT)
        .clamp(1, MAX_SAMPLE_LIMIT);
    let sql = format!(
        "SELECT timestamp_ns, step, rollout_id, group_id, sample_id, task, status, reward, \
         reward_pass, filter_reason, drop_reason, prompt_tokens, response_tokens, staleness \
         FROM rl.sample WHERE run_id = {} ORDER BY timestamp_ns DESC LIMIT {row_limit}",
        sql_literal(run_id)
    );
    let dataframe = query_local(&sql).await?;
    let samples = (0..dataframe.row_count())
        .filter_map(|row| parse_sample(&dataframe, row))
        .collect();
    Ok(RlSamplesResponse {
        run_id: run_id.to_string(),
        samples,
    })
}

async fn sampler_history(
    run_id: &str,
    limit: Option<usize>,
) -> ApiResult<RlSamplerHistoryResponse> {
    require_value("run_id", run_id)?;
    let row_limit = limit
        .unwrap_or(DEFAULT_SAMPLER_LIMIT)
        .clamp(1, MAX_SAMPLER_LIMIT);
    let sql = format!(
        "SELECT timestamp_ns, step, target, accepted, judged, trained, filtered, failed, \
         expired, in_flight FROM rl.sampler WHERE run_id = {} \
         ORDER BY timestamp_ns DESC LIMIT {row_limit}",
        sql_literal(run_id)
    );
    let dataframe = query_local(&sql).await?;
    let mut snapshots = (0..dataframe.row_count())
        .filter_map(|row| parse_sampler(&dataframe, row))
        .collect::<Vec<_>>();
    snapshots.reverse();
    Ok(RlSamplerHistoryResponse {
        run_id: run_id.to_string(),
        snapshots,
    })
}

async fn composition(
    run_id: &str,
    limit: Option<usize>,
    dimension: Option<&str>,
) -> ApiResult<RlCompositionResponse> {
    require_value("run_id", run_id)?;
    let dimension = match normalize_composition_dimension(dimension) {
        Ok(value) => value,
        Err(message) => return Err(ApiError::bad_request(message)),
    };
    let row_limit = limit
        .unwrap_or(DEFAULT_COMPOSITION_LIMIT)
        .clamp(1, MAX_COMPOSITION_LIMIT);
    let sql = format!(
        "SELECT step, task, category, status, filter_reason FROM rl.sample WHERE run_id = {} \
         ORDER BY timestamp_ns DESC LIMIT {row_limit}",
        sql_literal(run_id)
    );
    let dataframe = query_local(&sql).await?;
    let mut totals: BTreeMap<String, i64> = BTreeMap::new();
    let mut per_step: BTreeMap<i64, BTreeMap<String, i64>> = BTreeMap::new();
    let mut sample_count = 0_i64;
    for row in 0..dataframe.row_count() {
        let column = match dimension {
            "status" => "status",
            "filter_reason" => "filter_reason",
            "category" => "category",
            _ => "task",
        };
        let key = non_empty(text_at(&dataframe, column, row));
        *totals.entry(key.clone()).or_default() += 1;
        let step = dataframe.scalar_i64("step", row).unwrap_or(-1);
        *per_step.entry(step).or_default().entry(key).or_default() += 1;
        sample_count += 1;
    }
    let buckets = rank_buckets(dimension, totals, sample_count);
    // Keep only the bucket keys that survived truncation so the stacked
    // per-step view uses the same legend as the aggregate view.
    let retained = buckets
        .iter()
        .map(|bucket| bucket.key.clone())
        .collect::<BTreeSet<_>>();
    let steps = per_step
        .into_iter()
        .map(|(step, mut counts)| {
            let step_total = counts.values().sum::<i64>();
            if !retained.is_empty() {
                let folded = counts
                    .keys()
                    .filter(|key| !retained.contains(*key))
                    .cloned()
                    .collect::<Vec<_>>();
                let other = folded
                    .iter()
                    .filter_map(|key| counts.remove(key))
                    .sum::<i64>();
                if other > 0 {
                    *counts.entry("other".to_string()).or_default() += other;
                }
            }
            RlCompositionStep {
                step,
                sample_count: step_total,
                buckets: rank_buckets(dimension, counts, step_total),
            }
        })
        .collect();
    Ok(RlCompositionResponse {
        run_id: run_id.to_string(),
        sample_count,
        buckets,
        steps,
    })
}

/// Sort counts by descending size and fold the long tail into `other`.
/// Blank labels become `unknown`, so a sample with no task does not turn into an
/// unnamed row or bucket.
fn non_empty(value: Option<String>) -> String {
    match value {
        Some(text) if !text.trim().is_empty() => text,
        _ => "unknown".to_string(),
    }
}

fn rank_buckets(
    dimension: &str,
    counts: BTreeMap<String, i64>,
    total: i64,
) -> Vec<RlCompositionBucket> {
    let share = |count: i64| {
        if total > 0 {
            count as f64 / total as f64
        } else {
            0.0
        }
    };
    let mut buckets = counts
        .into_iter()
        .map(|(key, count)| RlCompositionBucket {
            dimension: dimension.to_string(),
            key,
            count,
            share: share(count),
        })
        .collect::<Vec<_>>();
    buckets.sort_by(|left, right| {
        right
            .count
            .cmp(&left.count)
            .then_with(|| left.key.cmp(&right.key))
    });
    if buckets.len() > MAX_COMPOSITION_BUCKETS {
        let remainder = buckets.split_off(MAX_COMPOSITION_BUCKETS - 1);
        let other_count = remainder.iter().map(|bucket| bucket.count).sum::<i64>();
        buckets.push(RlCompositionBucket {
            dimension: dimension.to_string(),
            key: "other".to_string(),
            count: other_count,
            share: share(other_count),
        });
    }
    buckets
}

/// Number of histogram slices, matching the nine-bucket convention: all-fail,
/// seven partial slices, all-pass.
const PASS_BUCKETS: usize = 9;

/// Which slice a prompt's pass rate falls into. The two ends are exact, so
/// bucket 0 and bucket 8 line up with the `sampler.pass_zero_ratio` and
/// `sampler.pass_one_ratio` gauges instead of approximating them.
fn pass_bucket(rate: f64) -> usize {
    // A NaN rate is unreachable from `build_pass_histogram`, but treat it as a
    // failure rather than letting it fall through to a partial slice.
    if rate.is_nan() || rate <= 0.0 {
        return 0;
    }
    if rate >= 1.0 {
        return PASS_BUCKETS - 1;
    }
    // Seven even slices across the open interval between the two ends.
    let slice = (rate * 7.0).ceil() as usize;
    slice.clamp(1, 7)
}

fn pass_bucket_label(index: usize) -> String {
    match index {
        0 => "0 (all fail)".to_string(),
        8 => "1 (all pass)".to_string(),
        other => {
            let low = (other - 1) as f64 / 7.0;
            let high = other as f64 / 7.0;
            format!("{:.0}–{:.0}%", low * 100.0, high * 100.0)
        }
    }
}

/// Distribution of per-prompt pass rates over the most recent samples.
async fn pass_histogram(run_id: &str, limit: Option<usize>) -> ApiResult<RlPassHistogramResponse> {
    require_value("run_id", run_id)?;
    let row_limit = limit
        .unwrap_or(DEFAULT_COMPOSITION_LIMIT)
        .clamp(1, MAX_COMPOSITION_LIMIT);
    let sql = format!(
        "SELECT step, group_id, reward_pass FROM rl.sample WHERE run_id = {} \
         ORDER BY timestamp_ns DESC LIMIT {row_limit}",
        sql_literal(run_id)
    );
    let dataframe = query_local(&sql).await?;
    Ok(build_pass_histogram(run_id, &dataframe))
}

fn build_pass_histogram(run_id: &str, dataframe: &DataFrame) -> RlPassHistogramResponse {
    // Keyed by step as well as prompt, so a prompt resampled at a later step is
    // counted as its own outcome rather than merged with the earlier attempt.
    let mut prompts: BTreeMap<(i64, String), (i64, i64)> = BTreeMap::new();
    let mut rollout_count = 0_i64;
    for row in 0..dataframe.row_count() {
        let step = dataframe.scalar_i64("step", row).unwrap_or(-1);
        let group = non_empty(text_at(dataframe, "group_id", row));
        let entry = prompts.entry((step, group)).or_insert((0, 0));
        entry.0 += 1;
        if dataframe.scalar_boolish("reward_pass", row) {
            entry.1 += 1;
        }
        rollout_count += 1;
    }
    let mut counts = [0_i64; PASS_BUCKETS];
    let mut rate_sum = 0.0;
    for (rollouts, passed) in prompts.values() {
        let rate = if *rollouts > 0 {
            *passed as f64 / *rollouts as f64
        } else {
            0.0
        };
        counts[pass_bucket(rate)] += 1;
        rate_sum += rate;
    }
    let prompt_count = prompts.len() as i64;
    let buckets = counts
        .iter()
        .enumerate()
        .map(|(index, prompts_in_bucket)| RlPassBucket {
            index: index as i64,
            label: pass_bucket_label(index),
            prompts: *prompts_in_bucket,
            share: if prompt_count > 0 {
                *prompts_in_bucket as f64 / prompt_count as f64
            } else {
                0.0
            },
        })
        .collect();
    RlPassHistogramResponse {
        run_id: run_id.to_string(),
        prompt_count,
        rollout_count,
        avg_pass_rate: if prompt_count > 0 {
            rate_sum / prompt_count as f64
        } else {
            0.0
        },
        buckets,
    }
}

/// Per-data-source sampling outcomes over the most recent samples, ordered by
/// volume so the heaviest sources come first.
async fn datasets(run_id: &str, limit: Option<usize>) -> ApiResult<RlDatasetsResponse> {
    require_value("run_id", run_id)?;
    let row_limit = limit
        .unwrap_or(DEFAULT_COMPOSITION_LIMIT)
        .clamp(1, MAX_COMPOSITION_LIMIT);
    let sql = format!(
        "SELECT step, task, category, status, reward_pass FROM rl.sample WHERE run_id = {} \
         ORDER BY timestamp_ns DESC LIMIT {row_limit}",
        sql_literal(run_id)
    );
    let dataframe = query_local(&sql).await?;
    Ok(build_datasets_response(run_id, &dataframe))
}

fn build_datasets_response(run_id: &str, dataframe: &DataFrame) -> RlDatasetsResponse {
    let mut rows: BTreeMap<String, RlDatasetRow> = BTreeMap::new();
    let mut judged: BTreeMap<String, i64> = BTreeMap::new();
    let mut sample_count = 0_i64;
    let mut latest_step = i64::MIN;
    for row in 0..dataframe.row_count() {
        let task = non_empty(text_at(dataframe, "task", row));
        let status = text_at(dataframe, "status", row).unwrap_or_default();
        let step = dataframe.scalar_i64("step", row).unwrap_or(-1);
        let entry = rows.entry(task.clone()).or_insert_with(|| RlDatasetRow {
            task: task.clone(),
            last_step: i64::MIN,
            ..Default::default()
        });
        if entry.category.is_empty() {
            entry.category = non_empty(text_at(dataframe, "category", row));
        }
        entry.samples += 1;
        entry.last_step = entry.last_step.max(step);
        match status.as_str() {
            "completed" => entry.completed += 1,
            "filtered" => entry.filtered += 1,
            "failed" => entry.failed += 1,
            // Anything still moving through the pipeline, such as `judging`.
            _ => entry.in_flight += 1,
        }
        // `reward_pass` only means anything once a judge has scored the sample.
        if matches!(status.as_str(), "completed" | "filtered") {
            *judged.entry(task.clone()).or_default() += 1;
            if dataframe.scalar_boolish("reward_pass", row) {
                entry.passed += 1;
            }
        }
        sample_count += 1;
        latest_step = latest_step.max(step);
    }
    let mut datasets = rows
        .into_values()
        .map(|mut row| {
            let scored = judged.get(&row.task).copied().unwrap_or(0);
            row.pass_rate = (scored > 0).then(|| row.passed as f64 / scored as f64);
            if row.last_step == i64::MIN {
                row.last_step = -1;
            }
            row
        })
        .collect::<Vec<_>>();
    // Heaviest source first, then alphabetically so the order is stable.
    datasets.sort_by(|left, right| {
        right
            .samples
            .cmp(&left.samples)
            .then_with(|| left.task.cmp(&right.task))
    });
    RlDatasetsResponse {
        run_id: run_id.to_string(),
        sample_count,
        latest_step: if latest_step == i64::MIN {
            -1
        } else {
            latest_step
        },
        datasets,
    }
}

async fn staleness(run_id: &str, limit: Option<usize>) -> ApiResult<RlStalenessResponse> {
    require_value("run_id", run_id)?;
    let row_limit = limit
        .unwrap_or(DEFAULT_STALENESS_LIMIT)
        .clamp(1, MAX_STALENESS_LIMIT);
    let sql = format!(
        "SELECT staleness, prompt_tokens, response_tokens FROM rl.sample \
         WHERE run_id = {} ORDER BY timestamp_ns DESC LIMIT {row_limit}",
        sql_literal(run_id)
    );
    let dataframe = query_local(&sql).await?;
    Ok(build_staleness_response(run_id, &dataframe))
}

async fn benchmarks(run_id: &str, limit: Option<usize>) -> ApiResult<RlBenchmarksResponse> {
    require_value("run_id", run_id)?;
    let row_limit = limit
        .unwrap_or(DEFAULT_BENCHMARK_LIMIT)
        .clamp(1, MAX_BENCHMARK_LIMIT);
    let sql = format!(
        "SELECT name, step, score, sample_count, version, harness, aggregation, timestamp_ns \
         FROM rl.benchmark \
         WHERE run_id = {} ORDER BY timestamp_ns DESC LIMIT {row_limit}",
        sql_literal(run_id)
    );
    let dataframe = query_local(&sql).await?;
    let mut points = (0..dataframe.row_count())
        .filter_map(|row| parse_benchmark(&dataframe, row))
        .collect::<Vec<_>>();
    points.reverse();
    Ok(RlBenchmarksResponse {
        run_id: run_id.to_string(),
        points,
    })
}

async fn events(run_id: &str, limit: Option<usize>) -> ApiResult<RlEventsResponse> {
    require_value("run_id", run_id)?;
    let row_limit = limit
        .unwrap_or(DEFAULT_EVENTS_LIMIT)
        .clamp(1, MAX_EVENTS_LIMIT);
    let status = status(run_id).await?;
    let series = series(run_id, "hardware.restart_count", Some(64), None)
        .await
        .unwrap_or_else(|_| RlSeriesResponse {
            run_id: run_id.to_string(),
            ..Default::default()
        });
    let benchmarks = benchmarks(run_id, Some(200))
        .await
        .unwrap_or_else(|_| RlBenchmarksResponse {
            run_id: run_id.to_string(),
            points: Vec::new(),
        });
    let mut events = synthesize_events(&status.run, &series, status.sampler.as_ref(), &benchmarks);
    // Operator notices are optional: the table only exists once something has
    // filed one, so a missing table must not fail the whole feed.
    events.extend(
        operator_notices(run_id, row_limit)
            .await
            .unwrap_or_default(),
    );
    events.sort_by(|left, right| {
        right
            .timestamp_ns
            .cmp(&left.timestamp_ns)
            .then_with(|| right.step.cmp(&left.step))
            .then_with(|| left.message.cmp(&right.message))
    });
    events.truncate(row_limit);
    Ok(RlEventsResponse {
        run_id: run_id.to_string(),
        events,
    })
}

async fn operator_notices(run_id: &str, limit: usize) -> ApiResult<Vec<RlEvent>> {
    let sql = format!(
        "SELECT timestamp_ns, step, level, kind, message FROM rl.notice \
         WHERE run_id = {} OR run_id = '' ORDER BY timestamp_ns DESC LIMIT {limit}",
        sql_literal(run_id)
    );
    Ok(parse_notice_events(&query_local(&sql).await?))
}

fn parse_notice_events(dataframe: &DataFrame) -> Vec<RlEvent> {
    (0..dataframe.row_count())
        .filter_map(|row| {
            let message = text_at(dataframe, "message", row)?;
            if message.trim().is_empty() {
                return None;
            }
            Some(RlEvent {
                timestamp_ns: dataframe.scalar_i64("timestamp_ns", row).unwrap_or(0),
                step: dataframe.scalar_i64("step", row).unwrap_or(-1),
                level: text_at(dataframe, "level", row)
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or_else(|| "info".to_string()),
                kind: text_at(dataframe, "kind", row)
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or_else(|| "operator".to_string()),
                message,
            })
        })
        .collect()
}

fn synthesize_events(
    run: &RlRunSummary,
    series: &RlSeriesResponse,
    sampler: Option<&RlSamplerSnapshot>,
    benchmarks: &RlBenchmarksResponse,
) -> Vec<RlEvent> {
    let mut events = Vec::new();
    if let Some((value, point)) = latest_metric_point(series, "hardware.restart_count") {
        if value > 0.0 {
            events.push(RlEvent {
                timestamp_ns: if point.timestamp_ns > 0 {
                    point.timestamp_ns
                } else {
                    run.timestamp_ns
                },
                step: if point.step >= 0 {
                    point.step
                } else {
                    run.global_step
                },
                level: "warning".into(),
                kind: "hardware.restart".into(),
                message: format!(
                    "Hardware restart count is {:.0} for run {}.",
                    value, run.run_id
                ),
            });
        }
    }
    if let Some(sampler) = sampler {
        if sampler.failed > 0 {
            events.push(RlEvent {
                timestamp_ns: if sampler.timestamp_ns > 0 {
                    sampler.timestamp_ns
                } else {
                    run.timestamp_ns
                },
                step: sampler.step,
                level: "warning".into(),
                kind: "sampler.failed".into(),
                message: format!(
                    "Sampler reported {} failed rollout(s) at step {} for run {}.",
                    sampler.failed, sampler.step, run.run_id
                ),
            });
        }
    }
    if let Some(point) = pinned_recent_benchmark(benchmarks, run.global_step) {
        events.push(RlEvent {
            timestamp_ns: if point.timestamp_ns > 0 {
                point.timestamp_ns
            } else {
                run.timestamp_ns
            },
            step: point.step,
            level: "info".into(),
            kind: "benchmark.published".into(),
            message: format!(
                "Benchmark {} published {:.3} at step {} for run {}.",
                point.name, point.score, point.step, run.run_id
            ),
        });
    }
    events
}

fn latest_metric_point<'a>(
    series: &'a RlSeriesResponse,
    name: &str,
) -> Option<(f64, &'a RlMetricPoint)> {
    series
        .series
        .get(name)?
        .iter()
        .rev()
        .find(|point| point.value.is_finite())
        .map(|point| (point.value, point))
}

fn pinned_recent_benchmark(
    benchmarks: &RlBenchmarksResponse,
    global_step: i64,
) -> Option<&RlBenchmarkPoint> {
    let mut latest: BTreeMap<&str, &RlBenchmarkPoint> = BTreeMap::new();
    for point in &benchmarks.points {
        latest
            .entry(point.name.as_str())
            .and_modify(|current| {
                if point.step > current.step
                    || (point.step == current.step && point.timestamp_ns > current.timestamp_ns)
                {
                    *current = point;
                }
            })
            .or_insert(point);
    }
    latest
        .into_values()
        .filter(|point| point.step >= global_step.saturating_sub(20))
        .max_by(|left, right| {
            benchmark_priority(&left.name)
                .cmp(&benchmark_priority(&right.name))
                .then_with(|| left.step.cmp(&right.step))
                .then_with(|| left.name.cmp(&right.name))
        })
}

fn benchmark_priority(name: &str) -> i32 {
    let lowered = name.to_ascii_lowercase();
    if lowered.contains("accuracy") {
        3
    } else if lowered.contains("avg_pass") || lowered.contains("pass") {
        2
    } else if lowered.contains("online") {
        1
    } else {
        0
    }
}

fn build_staleness_response(run_id: &str, dataframe: &DataFrame) -> RlStalenessResponse {
    let defs = [
        ("0", 0_i64, Some(0_i64)),
        ("1", 1, Some(1)),
        ("2", 2, Some(2)),
        ("3", 3, Some(3)),
        ("4+", 4, None),
    ];
    let mut counts = [0_i64; 5];
    let mut tokens = [0_i64; 5];
    let mut sample_count = 0_i64;
    let mut staleness_sum = 0_i64;
    for row in 0..dataframe.row_count() {
        let staleness = dataframe.scalar_i64("staleness", row).unwrap_or(0).max(0);
        let sample_tokens = dataframe.scalar_i64("prompt_tokens", row).unwrap_or(0)
            + dataframe.scalar_i64("response_tokens", row).unwrap_or(0);
        let index = match staleness {
            0 => 0,
            1 => 1,
            2 => 2,
            3 => 3,
            _ => 4,
        };
        counts[index] += 1;
        tokens[index] += sample_tokens.max(0);
        sample_count += 1;
        staleness_sum += staleness;
    }
    let buckets = defs
        .iter()
        .enumerate()
        .map(|(index, (label, min, max))| RlStalenessBucket {
            label: (*label).to_string(),
            min_staleness: *min,
            max_staleness: *max,
            count: counts[index],
            share: if sample_count > 0 {
                counts[index] as f64 / sample_count as f64
            } else {
                0.0
            },
            tokens: tokens[index],
        })
        .collect();
    RlStalenessResponse {
        run_id: run_id.to_string(),
        sample_count,
        avg_staleness: if sample_count > 0 {
            staleness_sum as f64 / sample_count as f64
        } else {
            0.0
        },
        buckets,
    }
}

fn parse_benchmark(dataframe: &DataFrame, row: usize) -> Option<RlBenchmarkPoint> {
    Some(RlBenchmarkPoint {
        name: text_at(dataframe, "name", row)?,
        step: dataframe.scalar_i64("step", row).unwrap_or(-1),
        score: dataframe.scalar_f64("score", row)?,
        sample_count: dataframe.scalar_i64("sample_count", row).unwrap_or(0),
        version: text_at(dataframe, "version", row).unwrap_or_default(),
        timestamp_ns: dataframe.scalar_i64("timestamp_ns", row).unwrap_or(0),
        harness: text_at(dataframe, "harness", row).unwrap_or_default(),
        aggregation: text_at(dataframe, "aggregation", row).unwrap_or_default(),
    })
}

async fn about(run_id: &str) -> ApiResult<RlAboutResponse> {
    require_value("run_id", run_id)?;
    let status = status(run_id).await?;
    let sql = format!(
        "SELECT name, value FROM ( \
         SELECT name, value, timestamp_ns FROM rl.metric \
         WHERE run_id = {} AND name LIKE 'about.%' AND rank IN (-1, 0) \
         ORDER BY timestamp_ns DESC LIMIT 200 \
         ) recent",
        sql_literal(run_id)
    );
    let dataframe = query_local(&sql).await?;
    let mut about_metrics = BTreeMap::new();
    for row in 0..dataframe.row_count() {
        let Some(name) = text_at(&dataframe, "name", row) else {
            continue;
        };
        if about_metrics.contains_key(&name) {
            continue;
        }
        if let Some(value) = dataframe.scalar_f64("value", row) {
            about_metrics.insert(name, value);
        }
    }
    let metadata = parse_metadata_json(&status.run.metadata_json);
    Ok(RlAboutResponse {
        run: Some(status.run),
        about_metrics,
        metadata,
    })
}

fn parse_metadata_json(raw: &str) -> BTreeMap<String, serde_json::Value> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "{}" {
        return BTreeMap::new();
    }
    match serde_json::from_str::<serde_json::Value>(trimmed) {
        Ok(serde_json::Value::Object(map)) => map.into_iter().collect(),
        _ => BTreeMap::new(),
    }
}

async fn query_local(sql: &str) -> ApiResult<DataFrame> {
    if let Some(message) = crate::engine_lifecycle::engine_not_ready_message() {
        return Err(ApiError::service_unavailable(message));
    }
    cluster_fanout::fanout_query(sql, false, true, ClusterFanoutScope::Auto)
        .await
        .map(|outcome| outcome.dataframe)
        .map_err(ApiError::from)
}

fn parse_run(dataframe: &DataFrame, row: usize) -> Option<RlRunSummary> {
    Some(RlRunSummary {
        run_id: text_at(dataframe, "run_id", row)?,
        framework: text_at(dataframe, "framework", row).unwrap_or_default(),
        phase: text_at(dataframe, "phase", row).unwrap_or_default(),
        global_step: dataframe.scalar_i64("global_step", row).unwrap_or(-1),
        timestamp_ns: dataframe.scalar_i64("timestamp_ns", row).unwrap_or(0),
        start_time_ns: dataframe.scalar_i64("start_time_ns", row).unwrap_or(0),
        end_time_ns: dataframe.scalar_i64("end_time_ns", row).unwrap_or(0),
        samples_total: dataframe.scalar_i64("samples_total", row).unwrap_or(0),
        tokens_total: dataframe.scalar_i64("tokens_total", row).unwrap_or(0),
        job_id: text_at(dataframe, "job_id", row).unwrap_or_default(),
        config_hash: text_at(dataframe, "config_hash", row).unwrap_or_default(),
        metadata_json: text_at(dataframe, "metadata_json", row).unwrap_or_default(),
    })
}

fn parse_sampler(dataframe: &DataFrame, row: usize) -> Option<RlSamplerSnapshot> {
    if row >= dataframe.row_count() {
        return None;
    }
    Some(RlSamplerSnapshot {
        timestamp_ns: dataframe.scalar_i64("timestamp_ns", row).unwrap_or(0),
        step: dataframe.scalar_i64("step", row).unwrap_or(-1),
        target: dataframe.scalar_i64("target", row).unwrap_or(0),
        accepted: dataframe.scalar_i64("accepted", row).unwrap_or(0),
        judged: dataframe.scalar_i64("judged", row).unwrap_or(0),
        trained: dataframe.scalar_i64("trained", row).unwrap_or(0),
        filtered: dataframe.scalar_i64("filtered", row).unwrap_or(0),
        failed: dataframe.scalar_i64("failed", row).unwrap_or(0),
        expired: dataframe.scalar_i64("expired", row).unwrap_or(0),
        in_flight: dataframe.scalar_i64("in_flight", row).unwrap_or(0),
    })
}

fn parse_sample(dataframe: &DataFrame, row: usize) -> Option<RlSampleSummary> {
    Some(RlSampleSummary {
        timestamp_ns: dataframe.scalar_i64("timestamp_ns", row).unwrap_or(0),
        step: dataframe.scalar_i64("step", row).unwrap_or(-1),
        rollout_id: text_at(dataframe, "rollout_id", row)?,
        group_id: text_at(dataframe, "group_id", row).unwrap_or_default(),
        sample_id: text_at(dataframe, "sample_id", row).unwrap_or_default(),
        task: text_at(dataframe, "task", row).unwrap_or_default(),
        status: text_at(dataframe, "status", row).unwrap_or_default(),
        reward: dataframe
            .scalar_f64("reward", row)
            .filter(|value| value.is_finite()),
        reward_pass: dataframe.scalar_boolish("reward_pass", row),
        filter_reason: text_at(dataframe, "filter_reason", row).unwrap_or_default(),
        drop_reason: text_at(dataframe, "drop_reason", row).unwrap_or_default(),
        prompt_tokens: dataframe.scalar_i64("prompt_tokens", row).unwrap_or(0),
        response_tokens: dataframe.scalar_i64("response_tokens", row).unwrap_or(0),
        staleness: dataframe.scalar_i64("staleness", row).unwrap_or(0),
    })
}

fn text_at(dataframe: &DataFrame, column: &str, row: usize) -> Option<String> {
    let index = dataframe.col_index(column)?;
    let value = dataframe.cols.get(index)?.get(row);
    match value {
        Ele::Text(value) | Ele::Url(value) => Some(value),
        Ele::I32(value) => Some(value.to_string()),
        Ele::I64(value) => Some(value.to_string()),
        Ele::F32(value) => Some(value.to_string()),
        Ele::F64(value) => Some(value.to_string()),
        Ele::BOOL(value) => Some(value.to_string()),
        _ => None,
    }
}

fn require_value(name: &str, value: &str) -> ApiResult<()> {
    if value.trim().is_empty() {
        Err(ApiError::bad_request(format!("{name} must not be empty")))
    } else {
        Ok(())
    }
}

fn normalize_composition_dimension(dimension: Option<&str>) -> Result<&'static str, String> {
    match dimension.unwrap_or("task") {
        "task" => Ok("task"),
        "category" => Ok("category"),
        "status" => Ok("status"),
        "filter_reason" => Ok("filter_reason"),
        other => Err(format!(
            "dimension must be task, category, status, or filter_reason (got {other})"
        )),
    }
}

fn sql_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use probing_proto::prelude::Seq;

    #[test]
    fn sql_literals_escape_quotes() {
        assert_eq!(sql_literal("run'one"), "'run''one'");
    }

    #[test]
    fn parses_run_summary() {
        let data = DataFrame::new(
            [
                "run_id",
                "framework",
                "phase",
                "global_step",
                "timestamp_ns",
                "start_time_ns",
                "end_time_ns",
                "samples_total",
                "tokens_total",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
            vec![
                Seq::SeqText(vec!["run-1".into()]),
                Seq::SeqText(vec!["xtuner".into()]),
                Seq::SeqText(vec!["training".into()]),
                Seq::SeqI64(vec![4]),
                Seq::SeqI64(vec![10]),
                Seq::SeqI64(vec![1]),
                Seq::SeqI64(vec![0]),
                Seq::SeqI64(vec![64]),
                Seq::SeqI64(vec![1024]),
            ],
        );
        let run = parse_run(&data, 0).unwrap();
        assert_eq!(run.run_id, "run-1");
        assert_eq!(run.global_step, 4);
        assert_eq!(run.tokens_total, 1024);
    }

    #[test]
    fn parses_about_metadata_object() {
        let metadata = parse_metadata_json(r#"{"lr":0.0001,"model":"demo"}"#);
        assert_eq!(metadata.get("model").and_then(|v| v.as_str()), Some("demo"));
    }

    #[test]
    fn parses_operator_notices_and_defaults_blank_fields() {
        let data = DataFrame::new(
            ["timestamp_ns", "step", "level", "kind", "message"]
                .into_iter()
                .map(str::to_string)
                .collect(),
            vec![
                Seq::SeqI64(vec![20, 30, 40]),
                Seq::SeqI64(vec![7, -1, 9]),
                Seq::SeqText(vec!["warning".into(), "".into(), "info".into()]),
                Seq::SeqText(vec!["restart".into(), "".into(), "dataset".into()]),
                Seq::SeqText(vec![
                    "node drained".into(),
                    "defaults apply".into(),
                    "   ".into(),
                ]),
            ],
        );
        let events = parse_notice_events(&data);
        assert_eq!(events.len(), 2, "blank messages are dropped");
        assert_eq!(events[0].level, "warning");
        assert_eq!(events[0].kind, "restart");
        assert_eq!(events[0].step, 7);
        assert_eq!(events[1].level, "info");
        assert_eq!(events[1].kind, "operator");
    }

    #[test]
    fn rank_buckets_folds_long_tail_into_other() {
        let counts = (0..MAX_COMPOSITION_BUCKETS + 3)
            .map(|index| (format!("task-{index:02}"), (index as i64) + 1))
            .collect::<BTreeMap<_, _>>();
        let total = counts.values().sum::<i64>();
        let buckets = rank_buckets("task", counts, total);
        assert_eq!(buckets.len(), MAX_COMPOSITION_BUCKETS);
        let last = buckets.last().unwrap();
        assert_eq!(last.key, "other");
        // The four smallest counts (1..=4) are the ones that get folded.
        assert_eq!(last.count, 1 + 2 + 3 + 4);
        let summed = buckets.iter().map(|bucket| bucket.share).sum::<f64>();
        assert!((summed - 1.0).abs() < 1e-9, "shares must sum to 1");
    }

    #[test]
    fn rejects_unknown_composition_dimension() {
        let err = normalize_composition_dimension(Some("source")).unwrap_err();
        assert!(err.contains("dimension"));
        assert_eq!(normalize_composition_dimension(None).unwrap(), "task");
        assert_eq!(
            normalize_composition_dimension(Some("category")).unwrap(),
            "category"
        );
    }

    #[test]
    fn builds_staleness_buckets() {
        let data = DataFrame::new(
            ["staleness", "prompt_tokens", "response_tokens"]
                .into_iter()
                .map(str::to_string)
                .collect(),
            vec![
                Seq::SeqI64(vec![0, 1, 4, 7]),
                Seq::SeqI64(vec![10, 10, 10, 10]),
                Seq::SeqI64(vec![5, 5, 5, 5]),
            ],
        );
        let response = build_staleness_response("run", &data);
        assert_eq!(response.sample_count, 4);
        assert_eq!(response.buckets[0].count, 1);
        assert_eq!(response.buckets[1].count, 1);
        assert_eq!(response.buckets[4].count, 2);
        assert!((response.avg_staleness - 3.0).abs() < 1e-9);
    }

    #[test]
    fn bucket_width_covers_the_whole_span() {
        // 1000 steps into 192 buckets: 6 steps each covers 1152 >= 1000.
        assert_eq!(bucket_width(1, 1000, 192), 6);
        // Fewer steps than buckets degrades to one step per bucket, never zero.
        assert_eq!(bucket_width(1, 10, 192), 1);
        assert_eq!(bucket_width(5, 5, 192), 1);
        // A long run still fits: the width grows instead of dropping history.
        let width = bucket_width(0, 99_999, 200);
        assert_eq!(width, 500);
        assert!(
            width * 200 >= 100_000,
            "buckets must span every step of the run"
        );
    }

    #[test]
    fn bucketed_series_carries_the_range_and_lands_on_the_newest_step() {
        let data = DataFrame::new(
            [
                "name",
                "bucket",
                "avg_value",
                "min_value",
                "max_value",
                "spread_value",
                "samples",
                "last_step",
                "last_ts",
                "last_wall",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
            vec![
                Seq::SeqText(
                    ["reward.mean", "reward.mean", "time.step_s"]
                        .into_iter()
                        .map(str::to_string)
                        .collect(),
                ),
                Seq::SeqI64(vec![0, 1, 0]),
                Seq::SeqF64(vec![0.25, 0.75, 12.0]),
                Seq::SeqF64(vec![0.10, 0.60, 11.0]),
                Seq::SeqF64(vec![0.40, 0.90, 13.0]),
                Seq::SeqF64(vec![0.05, 0.07, 0.5]),
                Seq::SeqI64(vec![6, 6, 6]),
                Seq::SeqI64(vec![6, 12, 6]),
                Seq::SeqI64(vec![600, 1200, 600]),
                Seq::SeqF64(vec![6.0, 12.0, 6.0]),
            ],
        );
        let response = build_bucketed_series("run", &data, 6);
        assert_eq!(response.bucket_steps, 6);
        assert_eq!(response.series.len(), 2);
        let reward = &response.series["reward.mean"];
        assert_eq!(reward.len(), 2);
        assert_eq!(reward[0].value, 0.25);
        assert_eq!(reward[0].low, Some(0.10));
        assert_eq!(reward[0].high, Some(0.40));
        // Carried separately from the extremes: a client draws the spread and
        // reports the extremes, because only the former stays put as buckets widen.
        assert_eq!(reward[0].spread, Some(0.05));
        assert_eq!(reward[0].samples, 6);
        // A bucket is placed at its newest step, so a chart ends on real telemetry.
        assert_eq!(reward[1].step, 12);
        assert_eq!(reward[1].timestamp_ns, 1200);
    }

    #[test]
    fn raw_series_points_report_no_range() {
        // The raw path must not claim to summarise anything, or a client would
        // draw a band of width zero around every point.
        let point = RlMetricPoint {
            step: 3,
            value: 1.0,
            ..Default::default()
        };
        assert_eq!(point.low, None);
        assert_eq!(point.high, None);
        assert_eq!(point.samples, 0);
    }

    #[test]
    fn pass_buckets_keep_both_ends_exact() {
        // The ends must be exact so they match the pass_zero / pass_one gauges.
        assert_eq!(pass_bucket(0.0), 0);
        assert_eq!(pass_bucket(1.0), 8);
        // Anything above zero leaves bucket 0, anything below one leaves bucket 8.
        assert_eq!(pass_bucket(0.001), 1);
        assert_eq!(pass_bucket(0.999), 7);
        // Seven even slices in between, upper bound inclusive.
        assert_eq!(pass_bucket(1.0 / 7.0), 1);
        assert_eq!(pass_bucket(1.5 / 7.0), 2);
        assert_eq!(pass_bucket(6.0 / 7.0), 6);
        // Out-of-range input cannot land outside the histogram.
        assert_eq!(pass_bucket(-0.5), 0);
        assert_eq!(pass_bucket(1.5), 8);
        assert_eq!(pass_bucket(f64::NAN), 0);
    }

    #[test]
    fn pass_histogram_groups_rollouts_by_prompt() {
        let data = DataFrame::new(
            ["step", "group_id", "reward_pass"]
                .into_iter()
                .map(str::to_string)
                .collect(),
            vec![
                // Two prompts at step 1, one prompt at step 2.
                Seq::SeqI64(vec![1, 1, 1, 1, 1, 1, 2, 2]),
                Seq::SeqText(
                    ["g1", "g1", "g1", "g2", "g2", "g2", "g1", "g1"]
                        .into_iter()
                        .map(str::to_string)
                        .collect(),
                ),
                // g1@1 passes 3/3, g2@1 passes 0/3, g1@2 passes 1/2.
                Seq::SeqI64(vec![1, 1, 1, 0, 0, 0, 1, 0]),
            ],
        );
        let response = build_pass_histogram("run", &data);
        assert_eq!(response.rollout_count, 8);
        // The same prompt at a later step counts separately.
        assert_eq!(response.prompt_count, 3);
        assert_eq!(response.buckets.len(), PASS_BUCKETS);
        assert_eq!(response.buckets[0].prompts, 1, "g2 failed everything");
        assert_eq!(response.buckets[8].prompts, 1, "g1@1 passed everything");
        // 0.5 falls in the fourth partial slice.
        assert_eq!(response.buckets[4].prompts, 1);
        assert!((response.avg_pass_rate - (1.0 + 0.0 + 0.5) / 3.0).abs() < 1e-9);
        let shares = response
            .buckets
            .iter()
            .map(|bucket| bucket.share)
            .sum::<f64>();
        assert!((shares - 1.0).abs() < 1e-9, "shares must sum to 1");
    }

    #[test]
    fn pass_histogram_of_no_samples_is_empty_not_a_division_by_zero() {
        let data = DataFrame::new(
            ["step", "group_id", "reward_pass"]
                .into_iter()
                .map(str::to_string)
                .collect(),
            vec![
                Seq::SeqI64(vec![]),
                Seq::SeqText(vec![]),
                Seq::SeqI64(vec![]),
            ],
        );
        let response = build_pass_histogram("run", &data);
        assert_eq!(response.prompt_count, 0);
        assert_eq!(response.avg_pass_rate, 0.0);
        assert!(response.buckets.iter().all(|bucket| bucket.prompts == 0));
        assert!(response.buckets.iter().all(|bucket| bucket.share == 0.0));
    }

    #[test]
    fn aggregates_datasets_by_volume_with_pass_rate() {
        let data = DataFrame::new(
            ["step", "task", "category", "status", "reward_pass"]
                .into_iter()
                .map(str::to_string)
                .collect(),
            vec![
                Seq::SeqI64(vec![7, 7, 8, 8, 8, 9]),
                Seq::SeqText(
                    ["gsm8k", "gsm8k", "gsm8k", "aime", "aime", ""]
                        .into_iter()
                        .map(str::to_string)
                        .collect(),
                ),
                Seq::SeqText(
                    [
                        "reasoning",
                        "reasoning",
                        "reasoning",
                        "reasoning",
                        "reasoning",
                        "",
                    ]
                    .into_iter()
                    .map(str::to_string)
                    .collect(),
                ),
                Seq::SeqText(
                    [
                        "completed",
                        "completed",
                        "filtered",
                        "completed",
                        "judging",
                        "failed",
                    ]
                    .into_iter()
                    .map(str::to_string)
                    .collect(),
                ),
                Seq::SeqI64(vec![1, 0, 0, 1, 0, 0]),
            ],
        );
        let response = build_datasets_response("run", &data);
        assert_eq!(response.sample_count, 6);
        assert_eq!(response.latest_step, 9);

        // Heaviest source first.
        let gsm8k = &response.datasets[0];
        assert_eq!(gsm8k.task, "gsm8k");
        assert_eq!((gsm8k.samples, gsm8k.completed, gsm8k.filtered), (3, 2, 1));
        assert_eq!(gsm8k.last_step, 8);
        // One of three judged samples passed.
        assert!((gsm8k.pass_rate.unwrap() - 1.0 / 3.0).abs() < 1e-9);

        let aime = &response.datasets[1];
        assert_eq!((aime.samples, aime.completed, aime.in_flight), (2, 1, 1));
        // The sample still judging is excluded from the denominator.
        assert!((aime.pass_rate.unwrap() - 1.0).abs() < 1e-9);

        // A blank task is still reported, and a source with nothing judged has no rate.
        let unknown = &response.datasets[2];
        assert_eq!((unknown.task.as_str(), unknown.failed), ("unknown", 1));
        assert_eq!(unknown.pass_rate, None);
    }

    #[test]
    fn synthesizes_restart_and_sampler_events() {
        let run = RlRunSummary {
            run_id: "demo".into(),
            framework: "demo".into(),
            phase: "training".into(),
            global_step: 100,
            timestamp_ns: 50,
            start_time_ns: 1,
            end_time_ns: 0,
            samples_total: 0,
            tokens_total: 0,
            job_id: String::new(),
            config_hash: String::new(),
            metadata_json: String::new(),
        };
        let series = RlSeriesResponse {
            run_id: "demo".into(),
            series: BTreeMap::from([(
                "hardware.restart_count".into(),
                vec![RlMetricPoint {
                    step: 100,
                    wall_time_s: 1.0,
                    timestamp_ns: 40,
                    value: 2.0,
                    ..Default::default()
                }],
            )]),
            ..Default::default()
        };
        let sampler = RlSamplerSnapshot {
            timestamp_ns: 45,
            step: 100,
            target: 10,
            accepted: 8,
            judged: 7,
            trained: 6,
            filtered: 1,
            failed: 2,
            expired: 0,
            in_flight: 1,
        };
        let events = synthesize_events(
            &run,
            &series,
            Some(&sampler),
            &RlBenchmarksResponse::default(),
        );
        assert!(events.iter().any(|event| event.kind == "hardware.restart"));
        assert!(events.iter().any(|event| event.kind == "sampler.failed"));
    }

    #[test]
    fn synthesizes_recent_benchmark_event() {
        let run = RlRunSummary {
            run_id: "demo".into(),
            framework: "demo".into(),
            phase: "training".into(),
            global_step: 100,
            timestamp_ns: 50,
            start_time_ns: 1,
            end_time_ns: 0,
            samples_total: 0,
            tokens_total: 0,
            job_id: String::new(),
            config_hash: String::new(),
            metadata_json: String::new(),
        };
        let benchmarks = RlBenchmarksResponse {
            run_id: "demo".into(),
            points: vec![
                RlBenchmarkPoint {
                    name: "code@live".into(),
                    step: 90,
                    score: 0.9,
                    sample_count: 10,
                    version: "v1".into(),
                    timestamp_ns: 30,
                    harness: String::new(),
                    aggregation: String::new(),
                },
                RlBenchmarkPoint {
                    name: "math@accuracy".into(),
                    step: 95,
                    score: 0.42,
                    sample_count: 8,
                    version: "v1".into(),
                    timestamp_ns: 35,
                    harness: String::new(),
                    aggregation: String::new(),
                },
            ],
        };
        let events = synthesize_events(&run, &RlSeriesResponse::default(), None, &benchmarks);
        let event = events
            .iter()
            .find(|event| event.kind == "benchmark.published")
            .expect("benchmark event");
        assert!(event.message.contains("math@accuracy"));
    }
}
