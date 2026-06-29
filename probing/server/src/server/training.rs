//! Training observability: cross-rank ``train.step`` durations for straggler heatmaps.
//!
//! Local data is always cheap; cluster fan-out runs only when ``cluster=true``.

use std::collections::HashSet;

use axum::Json;
use probing_proto::prelude::*;
use serde::{Deserialize, Serialize};

use super::cluster_fanout;
use super::error::ApiResult;

const STEP_MATRIX_SQL: &str = r#"
SELECT
    s.attributes,
    s.name,
    s.time AS start_time,
    CAST((e.time - s.time) / 1000 AS DOUBLE) AS duration_us
FROM python.trace_event s
JOIN python.trace_event e
  ON s.span_id = e.span_id AND e.record_type = 'span_end'
WHERE s.record_type = 'span_start' AND s.name = 'train.step'
ORDER BY s.time ASC
"#;

const STEP_WINDOW: usize = 120;

#[derive(Debug, Deserialize)]
pub struct StepMatrixParams {
    pub limit: Option<usize>,
    pub cluster: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StepDurationSample {
    pub rank: i32,
    /// Display index (chronological, 0-based window into recent steps).
    pub local_step: i64,
    /// Original ``local_step`` from span attributes (may be wrong on legacy data).
    #[serde(default)]
    pub coord_step: i64,
    pub duration_ms: f64,
    pub host: String,
    pub addr: String,
}

#[derive(Debug, Serialize)]
pub struct StepMatrixResponse {
    pub samples: Vec<StepDurationSample>,
    pub rank_count: usize,
    pub step_count: usize,
    pub cluster: bool,
    pub nodes_queried: usize,
    pub nodes_failed: Vec<String>,
}

pub async fn get_step_matrix(
    axum::extract::Query(params): axum::extract::Query<StepMatrixParams>,
) -> ApiResult<Json<StepMatrixResponse>> {
    let step_window = params.limit.unwrap_or(STEP_WINDOW).clamp(1, 5_000);
    let cluster = params.cluster.unwrap_or(false);
    // Fetch all completed train.step pairs; aggregate to unique local_step in Rust.
    let sql = STEP_MATRIX_SQL.to_string();

    let fanout = cluster_fanout::fanout_query(
        &sql,
        cluster,
        true,
        cluster_fanout::ClusterFanoutScope::Auto,
    )
    .await?;
    let host = crate::report::get_hostname().unwrap_or_else(|_| "localhost".into());
    let addr = probing_core::core::cluster::local_addr_label();
    let samples =
        aggregate_step_samples(&parse_step_df(&fanout.dataframe, &host, &addr), step_window);

    let rank_count = samples.iter().map(|s| s.rank).collect::<HashSet<_>>().len();
    let step_count = samples
        .iter()
        .map(|s| s.local_step)
        .collect::<HashSet<_>>()
        .len();

    Ok(Json(StepMatrixResponse {
        samples,
        rank_count,
        step_count,
        cluster,
        nodes_queried: fanout.meta.nodes_queried,
        nodes_failed: fanout.meta.nodes_failed,
    }))
}

#[derive(Debug, Clone)]
struct RawStepRow {
    rank: i32,
    coord_step: i64,
    duration_ms: f64,
    span_name: String,
    source: String,
    host: String,
    addr: String,
    start_time: i64,
}

fn parse_step_df(df: &DataFrame, default_host: &str, default_addr: &str) -> Vec<RawStepRow> {
    if df.names.is_empty() || df.cols.is_empty() {
        return vec![];
    }

    let attrs_idx = df.names.iter().position(|n| n == "attributes").unwrap_or(0);
    let name_idx = df.names.iter().position(|n| n == "name");
    let time_idx = df.names.iter().position(|n| n == "start_time");
    let dur_idx = df
        .names
        .iter()
        .position(|n| n == "duration_us")
        .unwrap_or(1);
    let host_idx = df.names.iter().position(|n| n == "_host");
    let addr_idx = df.names.iter().position(|n| n == "_addr");
    let rows = df.cols.first().map(|c| c.len()).unwrap_or(0);

    let mut out = Vec::with_capacity(rows);
    for row in 0..rows {
        let attrs_str = ele_as_str(df.cols.get(attrs_idx).map(|c| c.get(row)));
        let duration_us = ele_as_f64(df.cols.get(dur_idx).map(|c| c.get(row)));
        let (rank, coord_step, source) = parse_attrs(&attrs_str);
        let span_name = name_idx
            .and_then(|i| df.cols.get(i).map(|c| ele_as_str(Some(c.get(row)))))
            .unwrap_or_default();
        let start_time = time_idx
            .and_then(|i| df.cols.get(i).map(|c| ele_as_i64(Some(c.get(row)))))
            .unwrap_or(0);
        let host = host_idx
            .and_then(|i| df.cols.get(i).map(|c| ele_as_str(Some(c.get(row)))))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| default_host.to_string());
        let addr = addr_idx
            .and_then(|i| df.cols.get(i).map(|c| ele_as_str(Some(c.get(row)))))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| default_addr.to_string());
        out.push(RawStepRow {
            rank,
            coord_step,
            duration_ms: duration_us / 1000.0,
            span_name,
            source,
            host,
            addr,
            start_time,
        });
    }
    out
}

fn span_preference_score(span_name: &str, source: &str) -> i32 {
    let mut score = 0;
    if span_name == "batch" {
        score += 10;
    }
    match source {
        "manual" => score += 5,
        "torch_probe" => score -= 5,
        _ => {}
    }
    score
}

fn aggregate_step_samples(rows: &[RawStepRow], window: usize) -> Vec<StepDurationSample> {
    use std::collections::HashMap;

    let mut by_rank: HashMap<i32, Vec<usize>> = HashMap::new();
    for (idx, row) in rows.iter().enumerate() {
        if row.duration_ms <= 0.0 {
            continue;
        }
        by_rank.entry(row.rank).or_default().push(idx);
    }

    let mut out = Vec::new();
    for (_rank, mut indices) in by_rank {
        indices.sort_by_key(|&i| rows[i].start_time);

        let preferred: Vec<usize> = indices
            .iter()
            .copied()
            .filter(|&i| span_preference_score(&rows[i].span_name, &rows[i].source) > 0)
            .collect();
        let picked: Vec<usize> = if preferred.is_empty() {
            indices
        } else if preferred.len() >= indices.len().max(1) / 3 {
            preferred
        } else {
            indices
        };

        let collapsed = collapse_coord_duplicates(rows, &picked);
        let start = collapsed.len().saturating_sub(window);
        for (idx, &row_idx) in collapsed[start..].iter().enumerate() {
            let row = &rows[row_idx];
            out.push(StepDurationSample {
                rank: normalize_rank(row.rank),
                local_step: idx as i64,
                coord_step: row.coord_step,
                duration_ms: row.duration_ms,
                host: row.host.clone(),
                addr: row.addr.clone(),
            });
        }
    }

    out.sort_by_key(|a| (a.rank, a.local_step));
    out
}

fn collapse_coord_duplicates(rows: &[RawStepRow], picked: &[usize]) -> Vec<usize> {
    let mut out: Vec<usize> = Vec::with_capacity(picked.len());
    for &idx in picked {
        let row = &rows[idx];
        if row.coord_step < 0 {
            out.push(idx);
            continue;
        }
        if let Some(&last_idx) = out.last() {
            let last = &rows[last_idx];
            if last.coord_step == row.coord_step {
                let row_score = span_preference_score(&row.span_name, &row.source);
                let last_score = span_preference_score(&last.span_name, &last.source);
                if row_score > last_score
                    || (row_score == last_score && row.duration_ms > last.duration_ms)
                {
                    if let Some(slot) = out.last_mut() {
                        *slot = idx;
                    }
                }
                continue;
            }
        }
        out.push(idx);
    }
    out
}

/// Single-process spans often carry ``rank: -1`` when ``RANK`` was unset; treat as 0.
fn normalize_rank(rank: i32) -> i32 {
    if rank < 0 {
        0
    } else {
        rank
    }
}

fn parse_attrs(raw: &str) -> (i32, i64, String) {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
        return (-1, -1, String::new());
    };
    let rank = normalize_rank(value.get("rank").and_then(json_i64).unwrap_or(-1) as i32);
    let coord_step = value
        .get("local_step")
        .or_else(|| value.get("global_step"))
        .and_then(json_i64)
        .or_else(|| {
            let micro = value.get("micro_step").and_then(json_i64)?;
            let batches = value
                .get("micro_batches")
                .and_then(json_i64)
                .unwrap_or(1)
                .max(1);
            Some(micro / batches)
        })
        .unwrap_or(-1);
    let source = value
        .get("source")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    (rank, coord_step, source)
}

fn json_i64(v: &serde_json::Value) -> Option<i64> {
    v.as_i64()
        .or_else(|| v.as_u64().map(|n| n as i64))
        .or_else(|| v.as_f64().map(|n| n as i64))
}

fn ele_as_str(v: Option<Ele>) -> String {
    match v {
        Some(Ele::Text(s)) => s,
        Some(Ele::I64(n)) => n.to_string(),
        Some(Ele::I32(n)) => n.to_string(),
        Some(Ele::F64(n)) => n.to_string(),
        Some(Ele::F32(n)) => n.to_string(),
        _ => String::new(),
    }
}

fn ele_as_i64(v: Option<Ele>) -> i64 {
    match v {
        Some(Ele::I64(n)) => n,
        Some(Ele::I32(n)) => n as i64,
        Some(Ele::F64(n)) => n as i64,
        Some(Ele::F32(n)) => n as i64,
        Some(Ele::Text(s)) => s.parse().unwrap_or(0),
        _ => 0,
    }
}

fn ele_as_f64(v: Option<Ele>) -> f64 {
    match v {
        Some(Ele::F64(n)) => n,
        Some(Ele::F32(n)) => n as f64,
        Some(Ele::I64(n)) => n as f64,
        Some(Ele::I32(n)) => n as f64,
        Some(Ele::Text(s)) => s.parse().unwrap_or(0.0),
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregate_normalizes_legacy_single_process_rank() {
        let rows = vec![RawStepRow {
            rank: -1,
            coord_step: 2,
            duration_ms: 88.0,
            span_name: "step".into(),
            source: "torch_probe".into(),
            host: "h".into(),
            addr: "a".into(),
            start_time: 10,
        }];
        let out = aggregate_step_samples(&rows, 120);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].rank, 0);
        assert_eq!(out[0].local_step, 0);
        assert!((out[0].duration_ms - 88.0).abs() < f64::EPSILON);
    }

    #[test]
    fn parse_legacy_rank_minus_one_as_zero() {
        let (rank, step, source) =
            parse_attrs(r#"{"rank":-1,"local_step":3,"source":"torch_probe"}"#);
        assert_eq!(rank, 0);
        assert_eq!(step, 3);
        assert_eq!(source, "torch_probe");
    }

    #[test]
    fn parse_train_step_attributes() {
        let (rank, step, source) = parse_attrs(r#"{"rank":3,"local_step":42,"source":"manual"}"#);
        assert_eq!(rank, 3);
        assert_eq!(step, 42);
        assert_eq!(source, "manual");
    }

    #[test]
    fn aggregate_prefers_manual_batch_span_per_coord_step() {
        let rows = vec![
            RawStepRow {
                rank: 0,
                coord_step: 5,
                duration_ms: 50.0,
                span_name: "step".into(),
                source: "torch_probe".into(),
                host: "h".into(),
                addr: "a".into(),
                start_time: 100,
            },
            RawStepRow {
                rank: 0,
                coord_step: 5,
                duration_ms: 120.0,
                span_name: "batch".into(),
                source: "manual".into(),
                host: "h".into(),
                addr: "a".into(),
                start_time: 101,
            },
            RawStepRow {
                rank: 0,
                coord_step: 3,
                duration_ms: 80.0,
                span_name: "batch".into(),
                source: "manual".into(),
                host: "h".into(),
                addr: "a".into(),
                start_time: 50,
            },
        ];
        let out = aggregate_step_samples(&rows, 120);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].local_step, 0);
        assert_eq!(out[0].coord_step, 3);
        assert_eq!(out[1].local_step, 1);
        assert_eq!(out[1].coord_step, 5);
        assert!((out[1].duration_ms - 120.0).abs() < f64::EPSILON);
    }

    #[test]
    fn aggregate_assigns_chronological_index_for_legacy_duplicate_coords() {
        let rows: Vec<RawStepRow> = (0..6)
            .map(|i| RawStepRow {
                rank: 0,
                coord_step: i % 2,
                duration_ms: 100.0 + i as f64,
                span_name: "batch".into(),
                source: "manual".into(),
                host: "h".into(),
                addr: "a".into(),
                start_time: i * 10,
            })
            .collect();
        let out = aggregate_step_samples(&rows, 120);
        assert_eq!(out.len(), 6);
        assert_eq!(out.last().unwrap().local_step, 5);
    }

    #[test]
    fn aggregate_keeps_recent_window() {
        let rows: Vec<RawStepRow> = (0..10)
            .map(|step| RawStepRow {
                rank: 0,
                coord_step: step,
                duration_ms: step as f64,
                span_name: "batch".into(),
                source: "manual".into(),
                host: "h".into(),
                addr: "a".into(),
                start_time: step * 100,
            })
            .collect();
        let out = aggregate_step_samples(&rows, 4);
        assert_eq!(out.len(), 4);
        assert_eq!(out[0].local_step, 0);
        assert_eq!(out[0].coord_step, 6);
        assert_eq!(out[3].local_step, 3);
        assert_eq!(out[3].coord_step, 9);
    }
}
