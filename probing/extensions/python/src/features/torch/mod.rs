//! Torch module profiling: flamegraph / JSON from `python.torch_trace`.
//!
//! HTTP entrypoints live in [`crate::extensions::torch`]; rendering uses
//! [`crate::features::flamegraph`].

use std::{collections::BTreeMap, collections::HashMap, collections::HashSet, thread};

use anyhow::{Context, Result};
use log::{error, warn};
use serde_json::json;

use probing_core::runtime::block_on;

use crate::extensions::python::PythonProbeDataSource;
use crate::features::flamegraph::{
    empty_torch_html, Flamegraph, FlamegraphKind, FlamegraphOptions,
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TorchMetric {
    Duration,
    DeltaMb,
    PeakMb,
}

impl TorchMetric {
    fn parse(raw: Option<&str>) -> Self {
        match raw.map(str::trim).filter(|s| !s.is_empty()) {
            Some("delta_mb" | "memory" | "delta" | "mem") => Self::DeltaMb,
            Some("peak_mb" | "peak") => Self::PeakMb,
            _ => Self::Duration,
        }
    }

    fn id(self) -> &'static str {
        match self {
            Self::Duration => "duration",
            Self::DeltaMb => "delta_mb",
            Self::PeakMb => "peak_mb",
        }
    }

    fn count_name(self) -> &'static str {
        match self {
            Self::Duration => "ns",
            Self::DeltaMb | Self::PeakMb => "MB",
        }
    }

    fn subtitle(self) -> &'static str {
        match self {
            Self::Duration => "Median post-hook duration · statistical sampling",
            Self::DeltaMb => {
                "Median pre→post allocated delta · CUDA global memory · statistical sampling"
            }
            Self::PeakMb => {
                "Median pre→post peak allocated delta · CUDA global memory · statistical sampling"
            }
        }
    }
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Frame {
    stage: String,
    module: String,
}

/// Recent training steps included in flamegraph aggregates (matches Training page window).
const TORCH_RECENT_STEP_FILTER: &str =
    "local_step >= GREATEST(COALESCE((SELECT max(local_step) FROM python.torch_trace), 0) - 99, 0)";

/// Raw post-hook duration rows for Rust-side median aggregation.
const TORCH_DURATION_ROWS_QUERY: &str = r#"
    SELECT module, stage, CAST(duration AS DOUBLE) AS duration
    FROM python.torch_trace
    WHERE module <> 'None'
      AND stage LIKE 'post %'
"#;

/// Post-hook rows including memory columns (may be absent on older mmap tables).
const TORCH_POST_ROWS_QUERY: &str = r#"
    SELECT module, stage,
      CAST(duration AS DOUBLE) AS duration,
      CAST(allocated_delta AS DOUBLE) AS allocated_delta,
      CAST(max_allocated_delta AS DOUBLE) AS max_allocated_delta,
      CAST(allocated AS DOUBLE) AS allocated,
      CAST(max_allocated AS DOUBLE) AS max_allocated
    FROM python.torch_trace
    WHERE module <> 'None'
      AND stage LIKE 'post %'
"#;

const TORCH_MEMORY_ROWS_QUERY: &str = r#"
    SELECT local_step, module, stage, allocated, max_allocated
    FROM python.torch_trace
    WHERE module <> 'None'
      AND (stage LIKE 'pre %' OR stage LIKE 'post %')
"#;

/// Legacy rows without delta columns: SQL join on pre/post allocated.
const TORCH_DELTA_JOIN_QUERY: &str = r#"
    SELECT post.module, post.stage,
      CAST(post.allocated AS DOUBLE) - CAST(pre.allocated AS DOUBLE) AS value
    FROM python.torch_trace pre
    INNER JOIN python.torch_trace post
      ON pre.local_step = post.local_step
      AND pre.module = post.module
      AND (
        (pre.stage = 'pre forward' AND post.stage = 'post forward')
        OR (pre.stage = 'pre step' AND post.stage = 'post step')
        OR (pre.stage = 'pre backward' AND post.stage = 'post backward')
      )
    WHERE post.module <> 'None'
      AND (CAST(post.allocated AS DOUBLE) - CAST(pre.allocated AS DOUBLE)) > 0
      AND post.local_step >= GREATEST(COALESCE((SELECT max(local_step) FROM python.torch_trace), 0) - 99, 0)
"#;

/// Legacy rows without delta columns: SQL join on pre/post peak allocated.
const TORCH_PEAK_JOIN_QUERY: &str = r#"
    SELECT post.module, post.stage,
      CAST(post.max_allocated AS DOUBLE) - CAST(pre.max_allocated AS DOUBLE) AS value
    FROM python.torch_trace pre
    INNER JOIN python.torch_trace post
      ON pre.local_step = post.local_step
      AND pre.module = post.module
      AND (
        (pre.stage = 'pre forward' AND post.stage = 'post forward')
        OR (pre.stage = 'pre step' AND post.stage = 'post step')
        OR (pre.stage = 'pre backward' AND post.stage = 'post backward')
      )
    WHERE post.module <> 'None'
      AND (CAST(post.max_allocated AS DOUBLE) - CAST(pre.max_allocated AS DOUBLE)) > 0
      AND post.local_step >= GREATEST(COALESCE((SELECT max(local_step) FROM python.torch_trace), 0) - 99, 0)
"#;

/// Map stored hook labels to flamegraph phase names.
fn normalize_post_stage(stage: &str) -> Option<&'static str> {
    match stage.trim() {
        "post forward" => Some("forward"),
        "post backward" => Some("backward"),
        "post step" => Some("step"),
        _ => None,
    }
}

fn is_leaf_module(module: &str, all_modules: &HashSet<String>) -> bool {
    !all_modules
        .iter()
        .any(|other| other != module && other.starts_with(&format!("{module}.")))
}

fn backward_module_names(rows: &[(String, String, f64)]) -> HashSet<String> {
    rows.iter()
        .filter_map(|(module, stage, _)| {
            normalize_post_stage(stage)
                .filter(|p| *p == "backward")
                .map(|_| module.clone())
        })
        .collect()
}

fn filter_backward_leaf_rows(rows: Vec<(String, String, f64)>) -> Vec<(String, String, f64)> {
    let backward_names = backward_module_names(&rows);
    if backward_names.is_empty() {
        return rows;
    }
    rows.into_iter()
        .filter(|(module, stage, _)| {
            normalize_post_stage(stage) != Some("backward")
                || is_leaf_module(module, &backward_names)
        })
        .collect()
}

/// Build backward folded lines from SQL rows (leaf modules + parent self-time adjustment).
fn build_backward_folded_lines(
    data: &probing_proto::types::DataFrame,
    duration_col: usize,
    to_units: fn(f64) -> u64,
) -> Vec<String> {
    let module_idx = data.names.iter().position(|n| n == "module").unwrap_or(0);
    let stage_idx = data.names.iter().position(|n| n == "stage").unwrap_or(1);
    let mut grouped: HashMap<String, Vec<f64>> = HashMap::new();
    let rows = data.cols.first().map(|c| c.len()).unwrap_or(0);
    for row in 0..rows {
        let module = data
            .cols
            .get(module_idx)
            .map(|c| parse_text_col(&c.get(row)))
            .unwrap_or_default();
        let stage = data
            .cols
            .get(stage_idx)
            .map(|c| parse_text_col(&c.get(row)))
            .unwrap_or_default();
        if !stage.contains("backward") {
            continue;
        }
        let value = data
            .cols
            .get(duration_col)
            .map(|c| parse_value_col(&c.get(row)))
            .unwrap_or(0.0);
        if value > 0.0 {
            grouped.entry(module).or_default().push(value);
        }
    }
    let backward_names: HashSet<String> = grouped.keys().cloned().collect();
    let rows = grouped
        .into_iter()
        .filter(|(module, _)| is_leaf_module(module, &backward_names))
        .map(|(module, values)| (module, "post backward".to_string(), median_f64(&values)))
        .filter(|(_, _, v)| *v > 0.0);
    build_folded_lines(rows, to_units, false)
}

fn ensure_backward_phase_lines(
    data: &probing_proto::types::DataFrame,
    duration_col: usize,
    lines: &mut Vec<String>,
) {
    if lines.iter().any(|l| l.starts_with("backward;")) {
        return;
    }
    let direct = build_backward_folded_lines(data, duration_col, value_to_ns);
    if !direct.is_empty() {
        warn!(
            "torch flamegraph: recovered {} backward line(s) via direct path",
            direct.len()
        );
    }
    if direct.is_empty() {
        warn!("torch flamegraph: no backward lines — check python.torch_trace post backward duration>0");
        return;
    }
    lines.extend(direct);
}

fn value_to_ns(seconds: f64) -> u64 {
    (seconds * 1_000_000_000.0).round() as u64
}

fn value_to_micro_mb(mb: f64) -> u64 {
    (mb * 1_000_000.0).round() as u64
}

/// Build folded stacks for the flamegraph: `phase;module;child;leaf <units>`.
///
/// Parent modules receive negative adjustments so the flamegraph shows self time at each
/// hierarchy level when children were also measured.
fn build_folded_lines(
    rows: impl IntoIterator<Item = (String, String, f64)>,
    to_units: fn(f64) -> u64,
    clamp_non_negative: bool,
) -> Vec<String> {
    let rows = filter_backward_leaf_rows(rows.into_iter().collect());
    let mut frames = BTreeMap::<Frame, f64>::default();

    for (module, stage, raw_value) in rows {
        let phase = match normalize_post_stage(&stage) {
            Some(p) => p,
            None => continue,
        };
        let value = if clamp_non_negative {
            raw_value.max(0.0)
        } else {
            raw_value
        };
        if value <= 0.0 {
            continue;
        }

        let frame = Frame {
            stage: phase.to_string(),
            module: module.clone(),
        };

        frames
            .entry(frame.clone())
            .and_modify(|total| *total += value)
            .or_insert(value);

        let mut parts = module.split('.').collect::<Vec<_>>();
        if parts.len() > 1 {
            parts.pop();
            let parent = Frame {
                stage: phase.to_string(),
                module: parts.join("."),
            };
            frames.entry(parent).and_modify(|total| *total -= value);
        }
    }

    let mut lines = Vec::new();
    for (frame, value) in frames {
        let adjusted = if clamp_non_negative {
            value.max(0.0)
        } else {
            value
        };
        if adjusted <= 0.0 {
            continue;
        }

        let mut line = frame.stage;
        line.push(';');
        for part in frame.module.split('.') {
            line.push_str(part);
            line.push(';');
        }

        let units = to_units(adjusted);
        if units == 0 {
            continue;
        }
        line.push_str(&format!(" {}", units));
        lines.push(line);
    }

    lines
}

fn query_profiling_impl(query: &str) -> Result<probing_proto::types::DataFrame> {
    let query = query.to_owned();
    block_on(async move {
        let engine = probing_core::ENGINE.read().await;
        let result = engine
            .async_query(&query)
            .await
            .context("Torch query failed")?;
        result.ok_or_else(|| anyhow::anyhow!("engine returned no dataframe for torch query"))
    })
    .map_err(anyhow::Error::new)?
}

fn run_torch_query(query: &str) -> Result<probing_proto::types::DataFrame> {
    let query = query.to_owned();
    thread::spawn(move || -> Result<probing_proto::types::DataFrame> {
        match query_profiling_impl(&query) {
            Ok(df) => return Ok(df),
            Err(e) => {
                log::debug!("Global engine torch query failed ({e}), trying minimal engine");
            }
        }
        let engine = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("failed to build tokio runtime")?
            .block_on(async {
                probing_core::create_engine()
                    .with_data_source(PythonProbeDataSource::create("python"))
                    .build()
                    .await
            })?;
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("failed to build tokio runtime")?;
        rt.block_on(async { engine.async_query(&query).await })?
            .ok_or_else(|| anyhow::anyhow!("minimal engine returned no dataframe for torch query"))
    })
    .join()
    .map_err(|_| anyhow::anyhow!("error joining thread"))?
}

fn parse_value_col(ele: &probing_proto::types::Ele) -> f64 {
    match ele {
        probing_proto::types::Ele::F32(x) => *x as f64,
        probing_proto::types::Ele::F64(x) => *x,
        probing_proto::types::Ele::I64(x) => *x as f64,
        probing_proto::types::Ele::I32(x) => *x as f64,
        probing_proto::types::Ele::Text(s) => s.parse::<f64>().unwrap_or(0.0),
        _ => 0.0,
    }
}

fn parse_i64_col(ele: &probing_proto::types::Ele) -> Option<i64> {
    match ele {
        probing_proto::types::Ele::I64(x) => Some(*x),
        probing_proto::types::Ele::I32(x) => Some(*x as i64),
        probing_proto::types::Ele::F64(x) => Some(*x as i64),
        probing_proto::types::Ele::F32(x) => Some(*x as i64),
        _ => None,
    }
}

fn parse_text_col(ele: &probing_proto::types::Ele) -> String {
    match ele {
        probing_proto::types::Ele::Text(s) => s.to_string(),
        _ => String::new(),
    }
}

fn median_f64(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = sorted.len() / 2;
    if sorted.len().is_multiple_of(2) {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    } else {
        sorted[mid]
    }
}

fn hook_phase(stage: &str) -> Option<(bool, String)> {
    if let Some(phase) = stage.strip_prefix("pre ") {
        return Some((true, phase.to_string()));
    }
    if let Some(phase) = stage.strip_prefix("post ") {
        return Some((false, phase.to_string()));
    }
    None
}

struct ProfilingResult {
    lines: Vec<String>,
    subtitle: String,
}

fn lines_from_value_rows(
    data: &probing_proto::types::DataFrame,
    value_col: usize,
    to_units: fn(f64) -> u64,
    clamp_non_negative: bool,
) -> Vec<String> {
    let module_idx = data.names.iter().position(|n| n == "module").unwrap_or(0);
    let stage_idx = data.names.iter().position(|n| n == "stage").unwrap_or(1);
    let mut grouped: HashMap<(String, String), Vec<f64>> = HashMap::new();
    let rows = data.cols.first().map(|c| c.len()).unwrap_or(0);
    for row in 0..rows {
        let module = data
            .cols
            .get(module_idx)
            .map(|c| parse_text_col(&c.get(row)))
            .unwrap_or_default();
        let stage = data
            .cols
            .get(stage_idx)
            .map(|c| parse_text_col(&c.get(row)))
            .unwrap_or_default();
        let value = data
            .cols
            .get(value_col)
            .map(|c| parse_value_col(&c.get(row)))
            .unwrap_or(0.0);
        if value > 0.0 {
            grouped.entry((module, stage)).or_default().push(value);
        }
    }
    let rows = grouped
        .into_iter()
        .map(|((module, stage), values)| (module, stage, median_f64(&values)))
        .filter(|(_, _, v)| *v > 0.0);
    build_folded_lines(rows, to_units, clamp_non_negative)
}

fn build_median_lines_from_post_rows(
    data: &probing_proto::types::DataFrame,
    value_col: usize,
    to_units: fn(f64) -> u64,
    clamp_non_negative: bool,
) -> Vec<String> {
    lines_from_value_rows(data, value_col, to_units, clamp_non_negative)
}

fn torch_rows_query(base: &str) -> String {
    if base.contains("local_step >=") {
        return base.to_string();
    }
    let trimmed = base.trim_end().trim_end_matches(';');
    if trimmed.to_uppercase().contains(" WHERE ") {
        format!("{trimmed} AND {TORCH_RECENT_STEP_FILTER}")
    } else {
        format!("{trimmed} WHERE {TORCH_RECENT_STEP_FILTER}")
    }
}

fn lines_from_agg_query(query: &str, to_units: fn(f64) -> u64) -> Vec<String> {
    match run_torch_query(&torch_rows_query(query)) {
        Ok(data) => lines_from_value_rows(&data, 2, to_units, true),
        Err(err) => {
            log::debug!("Torch aggregate query failed ({err})");
            Vec::new()
        }
    }
}

fn build_lines_from_memory_pairs(
    data: &probing_proto::types::DataFrame,
    use_peak: bool,
) -> Vec<String> {
    let mut pre_rows: HashMap<(i64, String, String), (f64, f64)> = HashMap::new();
    let mut deltas: Vec<(String, String, f64)> = Vec::new();

    for line in data.iter() {
        let step = match parse_i64_col(&line[0]) {
            Some(s) => s,
            None => continue,
        };
        let module = parse_text_col(&line[1]);
        let stage = parse_text_col(&line[2]);
        let allocated = parse_value_col(&line[3]);
        let max_allocated = parse_value_col(&line[4]);
        let (is_pre, phase) = match hook_phase(&stage) {
            Some(v) => v,
            None => continue,
        };

        if is_pre {
            pre_rows.insert((step, module, phase), (allocated, max_allocated));
        } else if let Some((pre_alloc, pre_max)) = pre_rows.get(&(step, module.clone(), phase)) {
            let delta = if use_peak {
                (max_allocated - *pre_max).max(0.0)
            } else {
                (allocated - *pre_alloc).max(0.0)
            };
            if delta > 0.0 {
                deltas.push((module, stage, delta));
            }
        }
    }

    let mut grouped: HashMap<(String, String), Vec<f64>> = HashMap::new();
    for (module, stage, delta) in deltas {
        grouped.entry((module, stage)).or_default().push(delta);
    }

    let rows = grouped
        .into_iter()
        .map(|((module, stage), values)| (module, stage, median_f64(&values)))
        .filter(|(_, _, v)| *v > 0.0);

    build_folded_lines(rows, value_to_micro_mb, true)
}

fn query_memory_lines(
    join_query: &str,
    snapshot_value_col: usize,
    use_peak: bool,
    metric: TorchMetric,
) -> Result<ProfilingResult> {
    let post_query = torch_rows_query(TORCH_POST_ROWS_QUERY);
    let mut lines = if let Ok(data) = run_torch_query(&post_query) {
        let primary_delta_col = if use_peak { 4 } else { 3 };
        let snapshot_col = if use_peak { 6 } else { 5 };
        let mut from_delta =
            build_median_lines_from_post_rows(&data, primary_delta_col, value_to_micro_mb, true);
        if from_delta.is_empty() {
            from_delta =
                build_median_lines_from_post_rows(&data, snapshot_col, value_to_micro_mb, true);
        }
        from_delta
    } else {
        Vec::new()
    };

    if lines.is_empty() {
        lines = lines_from_agg_query(join_query, value_to_micro_mb);
    }
    if lines.is_empty() {
        if let Ok(data) = run_torch_query(&torch_rows_query(TORCH_MEMORY_ROWS_QUERY)) {
            lines = build_lines_from_memory_pairs(&data, use_peak);
        }
    }

    if !lines.is_empty() {
        return Ok(ProfilingResult {
            lines,
            subtitle: metric.subtitle().to_string(),
        });
    }

    if let Ok(data) = run_torch_query(&post_query) {
        lines =
            build_median_lines_from_post_rows(&data, snapshot_value_col, value_to_micro_mb, true);
    }
    let subtitle = if use_peak {
        "Median post-hook peak GPU allocated (global MB) · hook deltas were zero · CUDA only"
    } else {
        "Median post-hook GPU allocated (global MB) · hook deltas were zero · CUDA only"
    };
    Ok(ProfilingResult {
        lines,
        subtitle: subtitle.to_string(),
    })
}

fn query_profiling(metric: TorchMetric) -> Result<ProfilingResult> {
    match metric {
        TorchMetric::Duration => {
            let post_query = torch_rows_query(TORCH_DURATION_ROWS_QUERY);
            let data = run_torch_query(&post_query)?;
            let duration_col = data.names.iter().position(|n| n == "duration").unwrap_or(2);
            let mut lines =
                build_median_lines_from_post_rows(&data, duration_col, value_to_ns, false);
            ensure_backward_phase_lines(&data, duration_col, &mut lines);
            Ok(ProfilingResult {
                lines,
                subtitle: metric.subtitle().to_string(),
            })
        }
        TorchMetric::DeltaMb => query_memory_lines(TORCH_DELTA_JOIN_QUERY, 5, false, metric),
        TorchMetric::PeakMb => query_memory_lines(TORCH_PEAK_JOIN_QUERY, 6, true, metric),
    }
}

fn torch_flamegraph_subtitle(metric: TorchMetric, lines: &[String]) -> String {
    let base = metric.subtitle().to_string();
    let has_forward = lines.iter().any(|l| l.starts_with("forward;"));
    let has_backward = lines.iter().any(|l| l.starts_with("backward;"));
    let has_step = lines.iter().any(|l| l.starts_with("step;"));
    let mut parts = vec![base];
    if has_forward && !has_backward {
        parts.push(
            "No backward in flamegraph — need post backward with duration>0 (pre rows are ignored); use backward=on and rebuild probing if post.duration is 0"
                .to_string(),
        );
    }
    if has_forward && !has_step {
        parts.push(
            "No optimizer step samples in window (re-probe after fix, or check torch_trace post step rows)"
                .to_string(),
        );
    }
    parts.join(" · ")
}

fn torch_flamegraph_options(metric: TorchMetric, subtitle: &str) -> FlamegraphOptions {
    FlamegraphOptions {
        title: "Module performance".to_string(),
        count_name: metric.count_name().to_string(),
        kind: FlamegraphKind::TorchModule,
        subtitle: subtitle.to_string(),
        metric: Some(metric.id().to_string()),
        profile: None,
    }
}

pub fn flamegraph() -> String {
    match query_profiling(TorchMetric::Duration) {
        Err(err) => {
            error!("Failed to query torch profiling data: {err}");
            empty_torch_html("Torch profiling data unavailable")
        }
        Ok(result) => {
            if result.lines.is_empty() {
                warn!("Torch profiling returned no samples; skipping flamegraph generation");
                return empty_torch_html("No torch profiling samples collected");
            }

            let subtitle = torch_flamegraph_subtitle(TorchMetric::Duration, &result.lines);
            match Flamegraph::from_folded_lines(&result.lines) {
                Some(fg) => {
                    fg.render_html(&torch_flamegraph_options(TorchMetric::Duration, &subtitle))
                }
                None => empty_torch_html("No torch profiling samples collected"),
            }
        }
    }
}

/// JSON payload for the web UI (`GET /apis/torchextension/flamegraph/json`).
pub fn flamegraph_json(metric: Option<&str>) -> String {
    let metric = TorchMetric::parse(metric);

    let empty = |msg: &str, subtitle: &str| {
        json!({
            "profile": "torch-module",
            "title": "Module performance",
            "subtitle": subtitle,
            "countName": metric.count_name(),
            "metric": metric.id(),
            "total": 0,
            "width": 1400.0,
            "frameHeight": 32.0,
            "frames": [],
            "emptyMessage": msg,
        })
        .to_string()
    };

    match query_profiling(metric) {
        Err(err) => {
            error!("Failed to query torch profiling data: {err}");
            empty("Torch profiling data unavailable", metric.subtitle())
        }
        Ok(result) => {
            let subtitle = torch_flamegraph_subtitle(metric, &result.lines);
            let opts = torch_flamegraph_options(metric, &subtitle);
            if result.lines.is_empty() {
                warn!("Torch profiling returned no samples; skipping flamegraph generation");
                let msg = match metric {
                    TorchMetric::Duration => "No torch profiling samples collected",
                    TorchMetric::DeltaMb | TorchMetric::PeakMb => {
                        "No GPU memory samples (CUDA only, or run more training steps)"
                    }
                };
                return empty(msg, &result.subtitle);
            }

            match Flamegraph::from_folded_lines(&result.lines) {
                Some(fg) => fg.json_payload(&opts),
                None => empty("No torch profiling samples collected", &result.subtitle),
            }
        }
    }
}

#[derive(Debug, Clone)]
struct TorchTraceRow {
    rank: i64,
    local_step: i64,
    module: String,
    stage: String,
    duration: f64,
    allocated_delta: f64,
    max_allocated_delta: f64,
    allocated: f64,
    max_allocated: f64,
}

fn col_index(names: &[String], candidates: &[&str]) -> Option<usize> {
    names.iter().position(|n| candidates.contains(&n.as_str()))
}

fn parse_rank_col(ele: &probing_proto::types::Ele) -> i64 {
    match ele {
        probing_proto::types::Ele::I64(x) => *x,
        probing_proto::types::Ele::I32(x) => *x as i64,
        probing_proto::types::Ele::F64(x) => *x as i64,
        probing_proto::types::Ele::F32(x) => *x as i64,
        probing_proto::types::Ele::Text(s) => s.parse().unwrap_or(0),
        _ => 0,
    }
}

fn parse_torch_trace_rows(data: &probing_proto::types::DataFrame) -> Vec<TorchTraceRow> {
    let nrows = data.cols.first().map(|c| c.len()).unwrap_or(0);
    if nrows == 0 {
        return Vec::new();
    }

    let rank_idx = col_index(&data.names, &["rank", "_rank"]).unwrap_or(0);
    let module_idx = col_index(&data.names, &["module"]).unwrap_or(1);
    let stage_idx = col_index(&data.names, &["stage"]).unwrap_or(2);
    let step_idx = col_index(&data.names, &["local_step", "step"]);
    let duration_idx = col_index(&data.names, &["duration"]);
    let alloc_delta_idx = col_index(&data.names, &["allocated_delta"]);
    let max_alloc_delta_idx = col_index(&data.names, &["max_allocated_delta"]);
    let allocated_idx = col_index(&data.names, &["allocated"]);
    let max_allocated_idx = col_index(&data.names, &["max_allocated"]);

    let mut out = Vec::with_capacity(nrows);
    for row in 0..nrows {
        let rank = data
            .cols
            .get(rank_idx)
            .map(|c| parse_rank_col(&c.get(row)))
            .unwrap_or(0);
        let local_step = step_idx
            .and_then(|idx| data.cols.get(idx).and_then(|c| parse_i64_col(&c.get(row))))
            .unwrap_or(0);
        let module = data
            .cols
            .get(module_idx)
            .map(|c| parse_text_col(&c.get(row)))
            .unwrap_or_default();
        let stage = data
            .cols
            .get(stage_idx)
            .map(|c| parse_text_col(&c.get(row)))
            .unwrap_or_default();
        let duration = duration_idx
            .and_then(|idx| data.cols.get(idx).map(|c| parse_value_col(&c.get(row))))
            .unwrap_or(0.0);
        let allocated_delta = alloc_delta_idx
            .and_then(|idx| data.cols.get(idx).map(|c| parse_value_col(&c.get(row))))
            .unwrap_or(0.0);
        let max_allocated_delta = max_alloc_delta_idx
            .and_then(|idx| data.cols.get(idx).map(|c| parse_value_col(&c.get(row))))
            .unwrap_or(0.0);
        let allocated = allocated_idx
            .and_then(|idx| data.cols.get(idx).map(|c| parse_value_col(&c.get(row))))
            .unwrap_or(0.0);
        let max_allocated = max_allocated_idx
            .and_then(|idx| data.cols.get(idx).map(|c| parse_value_col(&c.get(row))))
            .unwrap_or(0.0);

        out.push(TorchTraceRow {
            rank,
            local_step,
            module,
            stage,
            duration,
            allocated_delta,
            max_allocated_delta,
            allocated,
            max_allocated,
        });
    }
    out
}

fn distributed_duration_lines(rows: &[TorchTraceRow]) -> Vec<String> {
    let post_rows = rows
        .iter()
        .filter(|r| r.stage.starts_with("post ") && r.duration > 0.0)
        .map(|r| (r.module.clone(), r.stage.clone(), r.duration))
        .collect::<Vec<_>>();
    build_folded_lines(post_rows, value_to_ns, false)
}

fn distributed_memory_lines(rows: &[TorchTraceRow], use_peak: bool) -> Vec<String> {
    let mut value_rows = rows
        .iter()
        .filter(|r| r.stage.starts_with("post "))
        .filter_map(|r| {
            let delta = if use_peak {
                r.max_allocated_delta
            } else {
                r.allocated_delta
            };
            (delta > 0.0).then_some((r.module.clone(), r.stage.clone(), delta))
        })
        .collect::<Vec<_>>();

    if value_rows.is_empty() {
        let mut pre: HashMap<(String, String), (f64, f64)> = HashMap::new();
        for r in rows {
            let (is_pre, phase) = match hook_phase(&r.stage) {
                Some(v) => v,
                None => continue,
            };
            if is_pre {
                pre.insert((r.module.clone(), phase), (r.allocated, r.max_allocated));
            } else if let Some((pre_alloc, pre_max)) = pre.get(&(r.module.clone(), phase)) {
                let delta = if use_peak {
                    (r.max_allocated - *pre_max).max(0.0)
                } else {
                    (r.allocated - *pre_alloc).max(0.0)
                };
                if delta > 0.0 {
                    value_rows.push((r.module.clone(), r.stage.clone(), delta));
                }
            }
        }
    }

    if value_rows.is_empty() {
        value_rows = rows
            .iter()
            .filter(|r| r.stage.starts_with("post "))
            .filter_map(|r| {
                let snapshot = if use_peak {
                    r.max_allocated
                } else {
                    r.allocated
                };
                (snapshot > 0.0).then_some((r.module.clone(), r.stage.clone(), snapshot))
            })
            .collect();
    }

    build_folded_lines(value_rows, value_to_micro_mb, true)
}

/// Fold per-rank stacks at one training step; identical paths merge across ranks.
fn build_distributed_folded_lines(
    data: &probing_proto::types::DataFrame,
    metric: TorchMetric,
) -> (Vec<String>, i64, usize) {
    let rows = parse_torch_trace_rows(data);
    if rows.is_empty() {
        return (Vec::new(), 0, 0);
    }

    let local_step = rows.iter().map(|r| r.local_step).max().unwrap_or(0);
    let rank_count = rows.iter().map(|r| r.rank).collect::<HashSet<_>>().len();

    let mut by_rank: BTreeMap<i64, Vec<TorchTraceRow>> = BTreeMap::new();
    for row in rows {
        by_rank.entry(row.rank).or_default().push(row);
    }

    let mut all_lines = Vec::new();
    for (_rank, rank_rows) in by_rank {
        let lines = match metric {
            TorchMetric::Duration => distributed_duration_lines(&rank_rows),
            TorchMetric::DeltaMb => distributed_memory_lines(&rank_rows, false),
            TorchMetric::PeakMb => distributed_memory_lines(&rank_rows, true),
        };
        all_lines.extend(lines);
    }

    (all_lines, local_step, rank_count)
}

fn distributed_flamegraph_subtitle(
    metric: TorchMetric,
    lines: &[String],
    local_step: i64,
    rank_count: usize,
) -> String {
    let base = torch_flamegraph_subtitle(metric, lines);
    format!(
        "{base} · local_step {local_step} · {rank_count} ranks · SPMD snapshot (same step, merge identical stacks)"
    )
}

fn distributed_torch_flamegraph_options(metric: TorchMetric, subtitle: &str) -> FlamegraphOptions {
    FlamegraphOptions {
        title: "Distributed module performance".to_string(),
        count_name: metric.count_name().to_string(),
        kind: FlamegraphKind::TorchModule,
        subtitle: subtitle.to_string(),
        metric: Some(metric.id().to_string()),
        profile: Some("torch-distributed".to_string()),
    }
}

/// JSON flamegraph from a federated or local ``torch_trace`` snapshot at one ``local_step``.
pub fn distributed_flamegraph_json_from_df(
    data: &probing_proto::types::DataFrame,
    metric: Option<&str>,
) -> String {
    let metric = TorchMetric::parse(metric);

    let empty = |msg: &str, subtitle: &str| {
        json!({
            "profile": "torch-distributed",
            "title": "Distributed module performance",
            "subtitle": subtitle,
            "countName": metric.count_name(),
            "metric": metric.id(),
            "total": 0,
            "width": 1400.0,
            "frameHeight": 32.0,
            "frames": [],
            "emptyMessage": msg,
        })
        .to_string()
    };

    let (lines, local_step, rank_count) = build_distributed_folded_lines(data, metric);
    if lines.is_empty() {
        let subtitle = format!(
            "{} · no rows for local_step {}",
            metric.subtitle(),
            if local_step > 0 {
                local_step.to_string()
            } else {
                "(latest)".to_string()
            }
        );
        return empty(
            "No torch_trace rows for this step (enable torch profiling and run training)",
            &subtitle,
        );
    }

    let subtitle = distributed_flamegraph_subtitle(metric, &lines, local_step, rank_count);
    let opts = distributed_torch_flamegraph_options(metric, &subtitle);

    match Flamegraph::from_folded_lines(&lines) {
        Some(fg) => fg.json_payload(&opts),
        None => empty(
            "No torch_trace samples after folding",
            &format!("{subtitle} · rank_count={rank_count}"),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_post_stage_maps_hook_labels() {
        assert_eq!(normalize_post_stage("post forward"), Some("forward"));
        assert_eq!(normalize_post_stage("post backward"), Some("backward"));
        assert_eq!(normalize_post_stage("post step"), Some("step"));
        assert_eq!(normalize_post_stage("pre forward"), None);
    }

    #[test]
    fn torch_flamegraph_subtitle_warns_when_backward_missing() {
        let lines = vec!["forward;layer; 10000000".to_string()];
        let subtitle = torch_flamegraph_subtitle(TorchMetric::Duration, &lines);
        assert!(subtitle.contains("backward=on"));
        let with_back = vec![
            "forward;layer; 10000000".to_string(),
            "backward;layer; 2000000".to_string(),
            "step;SGD; 500000".to_string(),
        ];
        let ok = torch_flamegraph_subtitle(TorchMetric::Duration, &with_back);
        assert!(!ok.contains("No backward"));
    }

    #[test]
    fn build_folded_lines_uses_phase_and_module_hierarchy() {
        let lines = build_folded_lines(
            [
                (
                    "model.features".to_string(),
                    "post forward".to_string(),
                    0.008,
                ),
                (
                    "model.features.conv1".to_string(),
                    "post forward".to_string(),
                    0.005,
                ),
            ],
            value_to_ns,
            false,
        );

        assert_eq!(lines.len(), 2);
        assert!(lines
            .iter()
            .any(|l| l.starts_with("forward;model;features;conv1; 5000000")));
        assert!(lines
            .iter()
            .any(|l| l.starts_with("forward;model;features; 3000000")));
    }

    #[test]
    fn build_folded_lines_skips_pre_rows_and_zero_duration() {
        let lines = build_folded_lines(
            [
                ("model".to_string(), "pre forward".to_string(), 0.0),
                ("model".to_string(), "post forward".to_string(), 0.0),
            ],
            value_to_ns,
            false,
        );
        assert!(lines.is_empty());
    }

    #[test]
    fn build_backward_folded_lines_from_dataframe() {
        use probing_proto::types::{DataFrame, Seq};

        let df = DataFrame::new(
            vec!["module".into(), "stage".into(), "duration".into()],
            vec![
                Seq::SeqText(vec!["AlexNet".into(), "features.0".into()]),
                Seq::SeqText(vec!["post backward".into(), "post backward".into()]),
                Seq::SeqF64(vec![0.012, 0.004]),
            ],
        );
        let lines = build_backward_folded_lines(&df, 2, value_to_ns);
        assert!(lines.iter().any(|l| l.starts_with("backward;AlexNet;")));
        assert!(lines.iter().any(|l| l.starts_with("backward;features;0;")));
    }

    #[test]
    fn ensure_backward_phase_lines_recover_when_main_path_empty() {
        use probing_proto::types::{DataFrame, Seq};

        let df = DataFrame::new(
            vec!["module".into(), "stage".into(), "duration".into()],
            vec![
                Seq::SeqText(vec!["AlexNet".into()]),
                Seq::SeqText(vec!["post backward".into()]),
                Seq::SeqF64(vec![0.01]),
            ],
        );
        let mut lines = Vec::new();
        ensure_backward_phase_lines(&df, 2, &mut lines);
        assert!(lines.iter().any(|l| l.starts_with("backward;")));
    }

    #[test]
    fn build_folded_lines_includes_post_backward_phase() {
        let lines = build_folded_lines(
            [
                ("AlexNet".to_string(), "post backward".to_string(), 0.012),
                (
                    "AlexNet.features.0".to_string(),
                    "post backward".to_string(),
                    0.004,
                ),
                ("AlexNet".to_string(), "post forward".to_string(), 0.040),
                ("SGD".to_string(), "post step".to_string(), 0.008),
            ],
            value_to_ns,
            false,
        );
        assert!(lines.iter().any(|l| l.starts_with("backward;")));
        assert!(lines.iter().any(|l| l.starts_with("forward;")));
        assert!(lines.iter().any(|l| l.starts_with("step;")));
        assert!(Flamegraph::from_folded_lines(&lines).is_some());
    }

    #[test]
    fn build_folded_lines_separates_forward_and_step_phases() {
        let lines = build_folded_lines(
            [
                ("layer".to_string(), "post forward".to_string(), 0.01),
                ("Adam".to_string(), "post step".to_string(), 0.002),
            ],
            value_to_ns,
            false,
        );
        assert!(lines
            .iter()
            .any(|l| l.starts_with("forward;layer; 10000000")));
        assert!(lines.iter().any(|l| l.starts_with("step;Adam; 2000000")));
    }

    #[test]
    fn build_folded_lines_clamps_negative_memory_deltas() {
        let lines = build_folded_lines(
            [("layer".to_string(), "post forward".to_string(), -5.0)],
            value_to_micro_mb,
            true,
        );
        assert!(lines.is_empty());
    }

    #[test]
    fn build_folded_lines_memory_uses_micro_mb_units() {
        let lines = build_folded_lines(
            [("layer".to_string(), "post forward".to_string(), 1.5)],
            value_to_micro_mb,
            true,
        );
        assert!(lines
            .iter()
            .any(|l| l.starts_with("forward;layer; 1500000")));
    }

    #[test]
    fn metric_parse_aliases() {
        assert_eq!(TorchMetric::parse(None), TorchMetric::Duration);
        assert_eq!(TorchMetric::parse(Some("memory")), TorchMetric::DeltaMb);
        assert_eq!(TorchMetric::parse(Some("peak")), TorchMetric::PeakMb);
    }

    #[test]
    fn build_lines_from_memory_pairs_computes_positive_deltas() {
        use probing_proto::types::{DataFrame, Seq};

        let df = DataFrame::new(
            vec![
                "step".into(),
                "module".into(),
                "stage".into(),
                "allocated".into(),
                "max_allocated".into(),
            ],
            vec![
                Seq::SeqI64(vec![1, 1]),
                Seq::SeqText(vec!["layer".into(), "layer".into()]),
                Seq::SeqText(vec!["pre forward".into(), "post forward".into()]),
                Seq::SeqF64(vec![100.0, 102.5]),
                Seq::SeqF64(vec![100.0, 103.0]),
            ],
        );

        let lines = build_lines_from_memory_pairs(&df, false);
        assert!(lines
            .iter()
            .any(|l| l.starts_with("forward;layer; 2500000")));
    }

    #[test]
    fn torch_rows_query_appends_recent_step_window() {
        let q = torch_rows_query(TORCH_DURATION_ROWS_QUERY);
        assert!(q.contains("local_step >="));
        assert!(q.contains("max(local_step)"));
        assert!(q.contains("- 99, 0)"), "step 0 must remain queryable: {q}");
        assert!(q.contains("module <> 'None'"));
    }

    #[test]
    fn torch_rows_query_is_idempotent_when_filter_present() {
        let already_filtered =
            format!("SELECT 1 FROM python.torch_trace WHERE {TORCH_RECENT_STEP_FILTER}");
        assert_eq!(torch_rows_query(&already_filtered), already_filtered);
    }

    #[test]
    fn duration_query_uses_post_rows_only() {
        assert!(TORCH_DURATION_ROWS_QUERY.contains("duration"));
        assert!(TORCH_DURATION_ROWS_QUERY.contains("post %"));
        assert!(!TORCH_DURATION_ROWS_QUERY.contains("LEFT JOIN"));
    }

    #[test]
    fn memory_join_queries_use_local_step_not_step() {
        for sql in [TORCH_DELTA_JOIN_QUERY, TORCH_PEAK_JOIN_QUERY] {
            assert!(sql.contains("local_step"), "expected local_step in: {sql}");
            assert!(!sql.contains("pre.step"), "legacy step column in: {sql}");
            assert!(!sql.contains("post.step"), "legacy step column in: {sql}");
        }
        assert!(TORCH_MEMORY_ROWS_QUERY.contains("local_step"));
        assert!(!TORCH_MEMORY_ROWS_QUERY.contains("SELECT step,"));
    }

    #[test]
    fn lines_from_value_rows_median_per_module_stage() {
        use probing_proto::types::{DataFrame, Seq};

        let df = DataFrame::new(
            vec!["module".into(), "stage".into(), "duration".into()],
            vec![
                Seq::SeqText(vec!["layer".into(), "layer".into(), "layer".into()]),
                Seq::SeqText(vec![
                    "post forward".into(),
                    "post forward".into(),
                    "post forward".into(),
                ]),
                Seq::SeqF64(vec![0.004, 0.006, 0.010]),
            ],
        );

        let lines = lines_from_value_rows(&df, 2, value_to_ns, false);
        assert!(lines
            .iter()
            .any(|l| l.starts_with("forward;layer; 6000000")));
    }

    #[test]
    fn build_median_lines_duration_only_schema() {
        use probing_proto::types::{DataFrame, Seq};

        let df = DataFrame::new(
            vec!["module".into(), "stage".into(), "duration".into()],
            vec![
                Seq::SeqText(vec!["conv1".into()]),
                Seq::SeqText(vec!["post forward".into()]),
                Seq::SeqF64(vec![0.002]),
            ],
        );

        let lines = build_median_lines_from_post_rows(&df, 2, value_to_ns, false);
        assert!(lines
            .iter()
            .any(|l| l.starts_with("forward;conv1; 2000000")));
    }

    #[test]
    fn distributed_merge_identical_stacks_across_ranks() {
        use probing_proto::types::{DataFrame, Seq};

        let df = DataFrame::new(
            vec![
                "rank".into(),
                "module".into(),
                "stage".into(),
                "duration".into(),
                "local_step".into(),
            ],
            vec![
                Seq::SeqI64(vec![0, 1]),
                Seq::SeqText(vec!["layer".into(), "layer".into()]),
                Seq::SeqText(vec!["post forward".into(), "post forward".into()]),
                Seq::SeqF64(vec![0.01, 0.01]),
                Seq::SeqI64(vec![5, 5]),
            ],
        );

        let (lines, step, rank_count) = build_distributed_folded_lines(&df, TorchMetric::Duration);
        assert_eq!(step, 5);
        assert_eq!(rank_count, 2);
        assert_eq!(lines.len(), 2);
        let json = distributed_flamegraph_json_from_df(&df, None);
        assert!(json.contains("\"total\":20000000"));
    }

    #[test]
    fn distributed_split_different_module_paths() {
        use probing_proto::types::{DataFrame, Seq};

        let df = DataFrame::new(
            vec![
                "rank".into(),
                "module".into(),
                "stage".into(),
                "duration".into(),
                "local_step".into(),
            ],
            vec![
                Seq::SeqI64(vec![0, 1]),
                Seq::SeqText(vec!["layer.a".into(), "layer.b".into()]),
                Seq::SeqText(vec!["post forward".into(), "post forward".into()]),
                Seq::SeqF64(vec![0.01, 0.02]),
                Seq::SeqI64(vec![3, 3]),
            ],
        );

        let (lines, _, rank_count) = build_distributed_folded_lines(&df, TorchMetric::Duration);
        assert_eq!(rank_count, 2);
        assert!(lines.iter().any(|l| l.starts_with("forward;layer;a;")));
        assert!(lines.iter().any(|l| l.starts_with("forward;layer;b;")));
        let json = distributed_flamegraph_json_from_df(&df, None);
        assert!(json.contains("\"total\":30000000"));
    }

    #[test]
    fn distributed_flamegraph_json_profile_tag() {
        use probing_proto::types::{DataFrame, Seq};

        let df = DataFrame::new(
            vec![
                "rank".into(),
                "module".into(),
                "stage".into(),
                "duration".into(),
                "local_step".into(),
            ],
            vec![
                Seq::SeqI64(vec![0]),
                Seq::SeqText(vec!["m".into()]),
                Seq::SeqText(vec!["post forward".into()]),
                Seq::SeqF64(vec![0.001]),
                Seq::SeqI64(vec![1]),
            ],
        );

        let json = distributed_flamegraph_json_from_df(&df, None);
        assert!(json.contains("\"profile\":\"torch-distributed\""));
        assert!(json.contains("local_step 1"));
    }

    #[test]
    fn distributed_federation_rank_column() {
        use probing_proto::types::{DataFrame, Seq};

        let df = DataFrame::new(
            vec![
                "_rank".into(),
                "module".into(),
                "stage".into(),
                "duration".into(),
                "local_step".into(),
            ],
            vec![
                Seq::SeqI64(vec![2, 3]),
                Seq::SeqText(vec!["layer".into(), "layer".into()]),
                Seq::SeqText(vec!["post forward".into(), "post forward".into()]),
                Seq::SeqF64(vec![0.005, 0.005]),
                Seq::SeqI64(vec![9, 9]),
            ],
        );

        let (lines, step, rank_count) = build_distributed_folded_lines(&df, TorchMetric::Duration);
        assert_eq!(step, 9);
        assert_eq!(rank_count, 2);
        let json = distributed_flamegraph_json_from_df(&df, None);
        assert!(json.contains("\"total\":10000000"));
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn distributed_memory_delta_mb_merge() {
        use probing_proto::types::{DataFrame, Seq};

        let df = DataFrame::new(
            vec![
                "rank".into(),
                "module".into(),
                "stage".into(),
                "allocated_delta".into(),
                "local_step".into(),
            ],
            vec![
                Seq::SeqI64(vec![0, 1]),
                Seq::SeqText(vec!["layer".into(), "layer".into()]),
                Seq::SeqText(vec!["post forward".into(), "post forward".into()]),
                Seq::SeqF64(vec![1.0, 1.5]),
                Seq::SeqI64(vec![4, 4]),
            ],
        );

        let json = distributed_flamegraph_json_from_df(&df, Some("delta_mb"));
        assert!(json.contains("\"metric\":\"delta_mb\""));
        assert!(json.contains("\"total\":2500000"));
    }

    #[test]
    fn distributed_hierarchy_merges_shared_parent() {
        use probing_proto::types::{DataFrame, Seq};

        let df = DataFrame::new(
            vec![
                "rank".into(),
                "module".into(),
                "stage".into(),
                "duration".into(),
                "local_step".into(),
            ],
            vec![
                Seq::SeqI64(vec![0, 0, 1, 1]),
                Seq::SeqText(vec![
                    "model.layer".into(),
                    "model.layer.conv".into(),
                    "model.layer".into(),
                    "model.layer.conv".into(),
                ]),
                Seq::SeqText(vec![
                    "post forward".into(),
                    "post forward".into(),
                    "post forward".into(),
                    "post forward".into(),
                ]),
                Seq::SeqF64(vec![0.01, 0.006, 0.01, 0.004]),
                Seq::SeqI64(vec![2, 2, 2, 2]),
            ],
        );

        let json = distributed_flamegraph_json_from_df(&df, None);
        assert!(json.contains("\"frames\":["));
        assert!(!json.contains("\"emptyMessage\""));
        let payload: serde_json::Value = serde_json::from_str(&json).expect("json");
        assert_eq!(payload["profile"], "torch-distributed");
        assert!(payload["total"].as_u64().unwrap_or(0) > 0);
    }

    #[test]
    fn distributed_prefers_training_rank_over_federation_tag() {
        use probing_proto::types::{DataFrame, Seq};

        let df = DataFrame::new(
            vec![
                "rank".into(),
                "_rank".into(),
                "module".into(),
                "stage".into(),
                "duration".into(),
                "local_step".into(),
            ],
            vec![
                Seq::SeqI64(vec![3]),
                Seq::SeqI64(vec![99]),
                Seq::SeqText(vec!["layer".into()]),
                Seq::SeqText(vec!["post forward".into()]),
                Seq::SeqF64(vec![0.01]),
                Seq::SeqI64(vec![1]),
            ],
        );

        let (lines, _, rank_count) = build_distributed_folded_lines(&df, TorchMetric::Duration);
        assert_eq!(rank_count, 1);
        let json = distributed_flamegraph_json_from_df(&df, None);
        assert!(json.contains("\"total\":10000000"));
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn distributed_empty_dataframe_returns_empty_message() {
        use probing_proto::types::DataFrame;

        let df = DataFrame::new(vec![], vec![]);
        let json = distributed_flamegraph_json_from_df(&df, None);
        assert!(json.contains("\"emptyMessage\""));
        assert!(json.contains("\"total\":0"));
    }
}
