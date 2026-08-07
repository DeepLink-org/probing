use std::collections::HashMap;

use super::ApiClient;
use crate::utils::error::{AppError, Result};
use probing_proto::prelude::{DataFrame, Ele};

/// Latest per-device sample from `gpu.utilization` (memory + compute util merged).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GpuSnapshot {
    pub ts: i64,
    pub device_id: i32,
    pub backend: String,
    pub name: String,
    pub memory_model: String,
    pub chip: Option<String>,
    pub free_bytes: i64,
    pub total_bytes: i64,
    pub used_bytes: i64,
    pub mem_used_pct: f32,
    pub gpu_util_pct: Option<f32>,
    pub mem_controller_util_pct: Option<f32>,
    pub renderer_util_pct: Option<f32>,
    pub tiler_util_pct: Option<f32>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GpuHistorySample {
    pub mem_used_pct: f32,
    pub gpu_util_pct: f32,
}

impl ApiClient {
    /// Latest utilization row per `device_id` (supports multi-GPU / 8× nodes).
    pub async fn fetch_gpu_latest(&self) -> Result<Vec<GpuSnapshot>> {
        let df = self
            .execute_query(
                "SELECT ts, device_id, backend, name, memory_model, chip, \
                 free_bytes, total_bytes, used_bytes, mem_used_pct, gpu_util_pct, \
                 mem_controller_util_pct, renderer_util_pct, tiler_util_pct \
                 FROM gpu.utilization u \
                 WHERE u.ts = (SELECT MAX(ts) FROM gpu.utilization) \
                 ORDER BY device_id",
            )
            .await?;
        parse_gpu_snapshots(&df)
    }

    /// Recent history for all devices (`limit` rows total, grouped client-side by device_id).
    pub async fn fetch_gpu_history(
        &self,
        limit: usize,
    ) -> Result<HashMap<i32, Vec<GpuHistorySample>>> {
        let cap = limit.saturating_mul(16).max(limit);
        let df = self
            .execute_query(&format!(
                "SELECT device_id, mem_used_pct, gpu_util_pct, ts \
                 FROM gpu.utilization ORDER BY ts DESC LIMIT {cap}"
            ))
            .await?;
        parse_gpu_history(&df, limit)
    }
}

fn invalid_gpu_data(message: impl Into<String>) -> AppError {
    AppError::Api(format!("Invalid GPU metrics response: {}", message.into()))
}

fn required_columns(df: &DataFrame, names: &[&str]) -> Result<(usize, Vec<usize>)> {
    let rows = df.row_count();
    let indexes = names
        .iter()
        .map(|name| {
            let index = df
                .names
                .iter()
                .position(|n| n == name)
                .ok_or_else(|| invalid_gpu_data(format!("missing required column `{name}`")))?;
            let column = df
                .cols
                .get(index)
                .ok_or_else(|| invalid_gpu_data(format!("missing data for column `{name}`")))?;
            if column.len() != rows {
                return Err(invalid_gpu_data(format!(
                    "column `{name}` has {} rows, expected {rows}",
                    column.len()
                )));
            }
            Ok(index)
        })
        .collect::<Result<Vec<_>>>()?;
    Ok((rows, indexes))
}

fn cell(df: &DataFrame, row: usize, col: usize, name: &str) -> Result<Ele> {
    let value = df
        .cols
        .get(col)
        .ok_or_else(|| invalid_gpu_data(format!("missing data for column `{name}`")))?
        .get(row);
    if value == Ele::Nil {
        return Err(invalid_gpu_data(format!(
            "null or missing value in `{name}` at row {row}"
        )));
    }
    Ok(value)
}

fn ele_f32(e: Ele, name: &str) -> Result<f32> {
    match e {
        Ele::F32(v) => Ok(v),
        Ele::F64(v) => Ok(v as f32),
        Ele::I32(v) => Ok(v as f32),
        Ele::I64(v) => Ok(v as f32),
        other => Err(invalid_gpu_data(format!(
            "column `{name}` expected numeric data, got {other:?}"
        ))),
    }
}

fn ele_i64(e: Ele, name: &str) -> Result<i64> {
    match e {
        Ele::I64(v) => Ok(v),
        Ele::I32(v) => Ok(v as i64),
        other => Err(invalid_gpu_data(format!(
            "column `{name}` expected integer data, got {other:?}"
        ))),
    }
}

fn ele_i32(e: Ele, name: &str) -> Result<i32> {
    match e {
        Ele::I32(v) => Ok(v),
        Ele::I64(v) => i32::try_from(v).map_err(|_| {
            invalid_gpu_data(format!("column `{name}` value {v} is outside i32 range"))
        }),
        other => Err(invalid_gpu_data(format!(
            "column `{name}` expected integer data, got {other:?}"
        ))),
    }
}

fn ele_text(e: Ele, name: &str) -> Result<String> {
    match e {
        Ele::Text(s) => Ok(s),
        other => Err(invalid_gpu_data(format!(
            "column `{name}` expected text data, got {other:?}"
        ))),
    }
}

fn opt_pct(v: f32) -> Option<f32> {
    if v < 0.0 {
        None
    } else {
        Some(v)
    }
}

fn non_empty(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}

fn parse_gpu_snapshots(df: &DataFrame) -> Result<Vec<GpuSnapshot>> {
    const COLUMNS: &[&str] = &[
        "ts",
        "device_id",
        "backend",
        "name",
        "memory_model",
        "chip",
        "free_bytes",
        "total_bytes",
        "used_bytes",
        "mem_used_pct",
        "gpu_util_pct",
        "mem_controller_util_pct",
        "renderer_util_pct",
        "tiler_util_pct",
    ];
    let (rows, indexes) = required_columns(df, COLUMNS)?;
    let mut snapshots = Vec::with_capacity(rows);
    for row in 0..rows {
        let value = |position| cell(df, row, indexes[position], COLUMNS[position]);
        snapshots.push(GpuSnapshot {
            ts: ele_i64(value(0)?, COLUMNS[0])?,
            device_id: ele_i32(value(1)?, COLUMNS[1])?,
            backend: ele_text(value(2)?, COLUMNS[2])?,
            name: ele_text(value(3)?, COLUMNS[3])?,
            memory_model: ele_text(value(4)?, COLUMNS[4])?,
            chip: non_empty(ele_text(value(5)?, COLUMNS[5])?.trim().to_string()),
            free_bytes: ele_i64(value(6)?, COLUMNS[6])?,
            total_bytes: ele_i64(value(7)?, COLUMNS[7])?,
            used_bytes: ele_i64(value(8)?, COLUMNS[8])?,
            mem_used_pct: ele_f32(value(9)?, COLUMNS[9])?,
            gpu_util_pct: opt_pct(ele_f32(value(10)?, COLUMNS[10])?),
            mem_controller_util_pct: opt_pct(ele_f32(value(11)?, COLUMNS[11])?),
            renderer_util_pct: opt_pct(ele_f32(value(12)?, COLUMNS[12])?),
            tiler_util_pct: opt_pct(ele_f32(value(13)?, COLUMNS[13])?),
        });
    }
    Ok(snapshots)
}

fn parse_gpu_history(
    df: &DataFrame,
    per_device_limit: usize,
) -> Result<HashMap<i32, Vec<GpuHistorySample>>> {
    const COLUMNS: &[&str] = &["device_id", "mem_used_pct", "gpu_util_pct", "ts"];
    let (rows, indexes) = required_columns(df, COLUMNS)?;
    let mut map: HashMap<i32, Vec<GpuHistorySample>> = HashMap::new();

    for row in 0..rows {
        let device_id = ele_i32(cell(df, row, indexes[0], COLUMNS[0])?, COLUMNS[0])?;
        let sample = GpuHistorySample {
            mem_used_pct: ele_f32(cell(df, row, indexes[1], COLUMNS[1])?, COLUMNS[1])?,
            gpu_util_pct: opt_pct(ele_f32(cell(df, row, indexes[2], COLUMNS[2])?, COLUMNS[2])?)
                .unwrap_or(0.0),
        };
        let entry = map.entry(device_id).or_default();
        if entry.len() < per_device_limit {
            entry.push(sample);
        }
    }

    for samples in map.values_mut() {
        samples.reverse();
    }
    Ok(map)
}

#[cfg(test)]
pub fn gpu_device_label(device_id: i32, name: &str) -> String {
    let short = name.split_whitespace().next().unwrap_or(name);
    if name.len() > 24 {
        format!("GPU {device_id} · {short}…")
    } else {
        format!("GPU {device_id} · {name}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use probing_proto::prelude::Seq;

    #[test]
    fn gpu_history_rejects_missing_required_column() {
        let df = DataFrame::new(
            vec![
                "device_id".into(),
                "mem_used_pct".into(),
                "gpu_util_pct".into(),
            ],
            vec![
                Seq::SeqI32(vec![0]),
                Seq::SeqF32(vec![10.0]),
                Seq::SeqF32(vec![20.0]),
            ],
        );
        let err = parse_gpu_history(&df, 10).unwrap_err();
        assert!(err.to_string().contains("`ts`"));
    }

    #[test]
    fn gpu_history_rejects_wrong_value_type() {
        let df = DataFrame::new(
            vec![
                "device_id".into(),
                "mem_used_pct".into(),
                "gpu_util_pct".into(),
                "ts".into(),
            ],
            vec![
                Seq::SeqText(vec!["gpu-zero".into()]),
                Seq::SeqF32(vec![10.0]),
                Seq::SeqF32(vec![20.0]),
                Seq::SeqI64(vec![1]),
            ],
        );
        let err = parse_gpu_history(&df, 10).unwrap_err();
        assert!(err.to_string().contains("device_id"));
    }

    #[test]
    fn gpu_device_label_truncates_long_names() {
        let label = gpu_device_label(3, "NVIDIA A100-SXM4-80GB");
        assert!(label.starts_with("GPU 3 ·"));
    }
}
