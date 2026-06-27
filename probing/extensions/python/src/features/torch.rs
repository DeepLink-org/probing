use std::{collections::BTreeMap, collections::HashMap, thread};

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

/// Median stage duration on post-hook rows only (pre rows carry timing metadata, not duration).
const TORCH_DURATION_QUERY: &str = r#"
    SELECT module, stage, median(CAST(duration AS DOUBLE)) AS value
    FROM python.torch_trace
    WHERE module <> 'None'
      AND stage LIKE 'post %'
      AND CAST(duration AS DOUBLE) > 0
    GROUP BY module, stage
    ORDER BY stage, module;
"#;

/// Median post-hook allocated delta (pre→post pair, stored on post rows).
const TORCH_DELTA_QUERY: &str = r#"
    SELECT module, stage, median(CAST(allocated_delta AS DOUBLE)) AS value
    FROM python.torch_trace
    WHERE module <> 'None'
      AND stage LIKE 'post %'
      AND CAST(allocated_delta AS DOUBLE) > 0
    GROUP BY module, stage
    ORDER BY stage, module;
"#;

/// Median post-hook peak-allocated delta (pre→post pair, stored on post rows).
const TORCH_PEAK_QUERY: &str = r#"
    SELECT module, stage, median(CAST(max_allocated_delta AS DOUBLE)) AS value
    FROM python.torch_trace
    WHERE module <> 'None'
      AND stage LIKE 'post %'
      AND CAST(max_allocated_delta AS DOUBLE) > 0
    GROUP BY module, stage
    ORDER BY stage, module;
"#;

/// Fallback when hook deltas are zero: median global GPU allocated at post-hook.
const TORCH_ALLOCATED_SNAPSHOT_QUERY: &str = r#"
    SELECT module, stage, median(CAST(allocated AS DOUBLE)) AS value
    FROM python.torch_trace
    WHERE module <> 'None'
      AND stage LIKE 'post %'
      AND CAST(allocated AS DOUBLE) > 0
    GROUP BY module, stage
    ORDER BY stage, module;
"#;

/// Fallback when peak deltas are zero: median global peak allocated at post-hook.
const TORCH_MAX_SNAPSHOT_QUERY: &str = r#"
    SELECT module, stage, median(CAST(max_allocated AS DOUBLE)) AS value
    FROM python.torch_trace
    WHERE module <> 'None'
      AND stage LIKE 'post %'
      AND CAST(max_allocated AS DOUBLE) > 0
    GROUP BY module, stage
    ORDER BY stage, module;
"#;

const TORCH_MEMORY_ROWS_QUERY: &str = r#"
    SELECT step, module, stage, allocated, max_allocated
    FROM python.torch_trace
    WHERE module <> 'None'
      AND (stage LIKE 'pre %' OR stage LIKE 'post %')
"#;

/// Legacy rows without delta columns: SQL join on pre/post allocated.
const TORCH_DELTA_JOIN_QUERY: &str = r#"
    SELECT post.module, post.stage,
      median(CAST(post.allocated AS DOUBLE) - CAST(pre.allocated AS DOUBLE)) AS value
    FROM python.torch_trace pre
    INNER JOIN python.torch_trace post
      ON pre.step = post.step
      AND pre.module = post.module
      AND (
        (pre.stage = 'pre forward' AND post.stage = 'post forward')
        OR (pre.stage = 'pre step' AND post.stage = 'post step')
        OR (pre.stage = 'pre backward' AND post.stage = 'post backward')
      )
    WHERE post.module <> 'None'
      AND (CAST(post.allocated AS DOUBLE) - CAST(pre.allocated AS DOUBLE)) > 0
    GROUP BY post.module, post.stage
    ORDER BY post.stage, post.module;
"#;

/// Legacy rows without delta columns: SQL join on pre/post peak allocated.
const TORCH_PEAK_JOIN_QUERY: &str = r#"
    SELECT post.module, post.stage,
      median(CAST(post.max_allocated AS DOUBLE) - CAST(pre.max_allocated AS DOUBLE)) AS value
    FROM python.torch_trace pre
    INNER JOIN python.torch_trace post
      ON pre.step = post.step
      AND pre.module = post.module
      AND (
        (pre.stage = 'pre forward' AND post.stage = 'post forward')
        OR (pre.stage = 'pre step' AND post.stage = 'post step')
        OR (pre.stage = 'pre backward' AND post.stage = 'post backward')
      )
    WHERE post.module <> 'None'
      AND (CAST(post.max_allocated AS DOUBLE) - CAST(pre.max_allocated AS DOUBLE)) > 0
    GROUP BY post.module, post.stage
    ORDER BY post.stage, post.module;
"#;

/// Map stored hook labels to flamegraph phase names.
fn normalize_post_stage(stage: &str) -> Option<&'static str> {
    match stage {
        "post forward" => Some("forward"),
        "post backward" => Some("backward"),
        "post step" => Some("step"),
        _ => None,
    }
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
        Ok(result.unwrap_or_default())
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
        Ok(rt
            .block_on(async { engine.async_query(&query).await })?
            .unwrap_or_default())
    })
    .join()
    .map_err(|_| anyhow::anyhow!("error joining thread"))?
}

fn parse_value_col(ele: &probing_proto::types::Ele) -> f64 {
    match ele {
        probing_proto::types::Ele::F32(x) => *x as f64,
        probing_proto::types::Ele::F64(x) => *x,
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

fn lines_from_agg_query(query: &str) -> Vec<String> {
    match run_torch_query(query) {
        Ok(data) => {
            let rows = data.iter().map(|line| {
                let value = parse_value_col(&line[2]);
                (parse_text_col(&line[0]), parse_text_col(&line[1]), value)
            });
            build_folded_lines(rows, value_to_micro_mb, true)
        }
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
    delta_query: &str,
    join_query: &str,
    snapshot_query: &str,
    use_peak: bool,
    metric: TorchMetric,
) -> Result<ProfilingResult> {
    let mut lines = lines_from_agg_query(delta_query);
    if lines.is_empty() {
        lines = lines_from_agg_query(join_query);
    }
    if lines.is_empty() {
        if let Ok(data) = run_torch_query(TORCH_MEMORY_ROWS_QUERY) {
            lines = build_lines_from_memory_pairs(&data, use_peak);
        }
    }

    if !lines.is_empty() {
        return Ok(ProfilingResult {
            lines,
            subtitle: metric.subtitle().to_string(),
        });
    }

    let snapshot_lines = lines_from_agg_query(snapshot_query);
    let subtitle = if use_peak {
        "Median post-hook peak GPU allocated (global MB) · hook deltas were zero · CUDA only"
    } else {
        "Median post-hook GPU allocated (global MB) · hook deltas were zero · CUDA only"
    };
    Ok(ProfilingResult {
        lines: snapshot_lines,
        subtitle: subtitle.to_string(),
    })
}

fn query_profiling(metric: TorchMetric) -> Result<ProfilingResult> {
    match metric {
        TorchMetric::Duration => {
            let data = run_torch_query(TORCH_DURATION_QUERY)?;
            let rows = data.iter().map(|line| {
                let value = parse_value_col(&line[2]);
                (parse_text_col(&line[0]), parse_text_col(&line[1]), value)
            });
            let lines = build_folded_lines(rows, value_to_ns, false);
            Ok(ProfilingResult {
                lines,
                subtitle: metric.subtitle().to_string(),
            })
        }
        TorchMetric::DeltaMb => query_memory_lines(
            TORCH_DELTA_QUERY,
            TORCH_DELTA_JOIN_QUERY,
            TORCH_ALLOCATED_SNAPSHOT_QUERY,
            false,
            metric,
        ),
        TorchMetric::PeakMb => query_memory_lines(
            TORCH_PEAK_QUERY,
            TORCH_PEAK_JOIN_QUERY,
            TORCH_MAX_SNAPSHOT_QUERY,
            true,
            metric,
        ),
    }
}

fn torch_flamegraph_options(metric: TorchMetric, subtitle: &str) -> FlamegraphOptions {
    FlamegraphOptions {
        title: "Module performance".to_string(),
        count_name: metric.count_name().to_string(),
        kind: FlamegraphKind::TorchModule,
        subtitle: subtitle.to_string(),
        metric: Some(metric.id().to_string()),
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

            match Flamegraph::from_folded_lines(&result.lines) {
                Some(fg) => fg.render_html(&torch_flamegraph_options(
                    TorchMetric::Duration,
                    &result.subtitle,
                )),
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
            let opts = torch_flamegraph_options(metric, &result.subtitle);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_post_stage_maps_hook_labels() {
        assert_eq!(normalize_post_stage("post forward"), Some("forward"));
        assert_eq!(normalize_post_stage("post step"), Some("step"));
        assert_eq!(normalize_post_stage("pre forward"), None);
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
}
