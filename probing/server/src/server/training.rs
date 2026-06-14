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
    CAST((e.timestamp - s.timestamp) / 1000 AS DOUBLE) AS duration_us
FROM python.trace_event s
JOIN python.trace_event e
  ON s.span_id = e.span_id AND e.record_type = 'span_end'
WHERE s.record_type = 'span_start' AND s.kind = 'train.step'
ORDER BY s.timestamp DESC
"#;

#[derive(Debug, Deserialize)]
pub struct StepMatrixParams {
    pub limit: Option<usize>,
    pub cluster: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StepDurationSample {
    pub rank: i32,
    pub local_step: i64,
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
    let limit = params.limit.unwrap_or(500).clamp(1, 5_000);
    let cluster = params.cluster.unwrap_or(false);
    let sql = format!("{STEP_MATRIX_SQL} LIMIT {limit}");

    let fanout = cluster_fanout::fanout_query(&sql, cluster).await?;
    let host = cluster_fanout::local_host_label();
    let addr = cluster_fanout::local_addr_label();
    let samples = parse_step_df(&fanout.dataframe, &host, &addr);

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

fn parse_step_df(df: &DataFrame, default_host: &str, default_addr: &str) -> Vec<StepDurationSample> {
    if df.names.is_empty() || df.cols.is_empty() {
        return vec![];
    }

    let attrs_idx = df.names.iter().position(|n| n == "attributes").unwrap_or(0);
    let dur_idx = df
        .names
        .iter()
        .position(|n| n == "duration_us")
        .unwrap_or(1);
    let host_idx = df.names.iter().position(|n| n == "_probe_host");
    let addr_idx = df.names.iter().position(|n| n == "_probe_addr");
    let rows = df.cols.first().map(|c| c.len()).unwrap_or(0);

    let mut out = Vec::with_capacity(rows);
    for row in 0..rows {
        let attrs_str = ele_as_str(df.cols.get(attrs_idx).map(|c| c.get(row)));
        let duration_us = ele_as_f64(df.cols.get(dur_idx).map(|c| c.get(row)));
        let (rank, local_step) = parse_attrs(&attrs_str);
        let host = host_idx
            .and_then(|i| df.cols.get(i).map(|c| ele_as_str(Some(c.get(row)))))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| default_host.to_string());
        let addr = addr_idx
            .and_then(|i| df.cols.get(i).map(|c| ele_as_str(Some(c.get(row)))))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| default_addr.to_string());
        out.push(StepDurationSample {
            rank,
            local_step,
            duration_ms: duration_us / 1000.0,
            host,
            addr,
        });
    }
    out
}

fn parse_attrs(raw: &str) -> (i32, i64) {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
        return (-1, -1);
    };
    let rank = value.get("rank").and_then(json_i64).unwrap_or(-1) as i32;
    let local_step = value
        .get("local_step")
        .and_then(json_i64)
        .unwrap_or(-1);
    (rank, local_step)
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
    fn parse_train_step_attributes() {
        let (rank, step) = parse_attrs(r#"{"rank":3,"local_step":42}"#);
        assert_eq!(rank, 3);
        assert_eq!(step, 42);
    }
}
