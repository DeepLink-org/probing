use std::collections::HashMap;

use super::ApiClient;
use crate::utils::error::{AppError, Result};
use probing_proto::prelude::{DataFrame, Ele};

/// Static GPU device from `gpu.devices`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GpuDeviceRow {
    pub device_id: i32,
    pub backend: String,
    pub name: String,
    pub memory_model: String,
    pub chip: Option<String>,
    pub compute_capability: Option<String>,
    pub total_mem_bytes: i64,
}

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

fn is_gpu_table_missing(err: &AppError) -> bool {
    matches!(err, AppError::Api(msg)
        if msg.contains("gpu.") && msg.contains("not found"))
}

impl ApiClient {
    pub async fn fetch_gpu_devices(&self) -> Result<Vec<GpuDeviceRow>> {
        match self
            .execute_query(
                "SELECT device_id, backend, name, memory_model, chip, compute_capability, total_mem_bytes \
                 FROM gpu.devices ORDER BY device_id",
            )
            .await
        {
            Ok(df) => Ok(parse_gpu_devices(&df)),
            Err(e) if is_gpu_table_missing(&e) => Ok(vec![]),
            Err(e) => Err(e),
        }
    }

    /// Latest utilization row per `device_id` (supports multi-GPU / 8× nodes).
    pub async fn fetch_gpu_latest(&self) -> Result<Vec<GpuSnapshot>> {
        match self
            .execute_query(
                "SELECT ts, device_id, backend, name, memory_model, chip, \
                 free_bytes, total_bytes, used_bytes, mem_used_pct, gpu_util_pct, \
                 mem_controller_util_pct, renderer_util_pct, tiler_util_pct \
                 FROM gpu.utilization u \
                 WHERE u.ts = (SELECT MAX(ts) FROM gpu.utilization) \
                 ORDER BY device_id",
            )
            .await
        {
            Ok(df) => Ok(parse_gpu_snapshots(&df)),
            Err(e) if is_gpu_table_missing(&e) => Ok(vec![]),
            Err(e) => Err(e),
        }
    }

    /// Recent history for all devices (`limit` rows total, grouped client-side by device_id).
    pub async fn fetch_gpu_history(&self, limit: usize) -> Result<HashMap<i32, Vec<GpuHistorySample>>> {
        let cap = limit.saturating_mul(16).max(limit);
        match self
            .execute_query(&format!(
                "SELECT device_id, mem_used_pct, gpu_util_pct, ts \
                 FROM gpu.utilization ORDER BY ts DESC LIMIT {cap}"
            ))
            .await
        {
            Ok(df) => Ok(parse_gpu_history(&df, limit)),
            Err(e) if is_gpu_table_missing(&e) => Ok(HashMap::new()),
            Err(e) => Err(e),
        }
    }
}

fn col_index(df: &DataFrame, name: &str) -> Option<usize> {
    df.names.iter().position(|n| n == name)
}

fn cell(df: &DataFrame, row: usize, col: usize) -> Option<Ele> {
    df.cols.get(col).map(|c| c.get(row))
}

fn ele_f32(e: Ele) -> f32 {
    match e {
        Ele::F32(v) => v,
        Ele::F64(v) => v as f32,
        Ele::I32(v) => v as f32,
        Ele::I64(v) => v as f32,
        _ => 0.0,
    }
}

fn ele_i64(e: Ele) -> i64 {
    match e {
        Ele::I64(v) => v,
        Ele::I32(v) => v as i64,
        _ => 0,
    }
}

fn ele_i32(e: Ele) -> i32 {
    match e {
        Ele::I32(v) => v,
        Ele::I64(v) => v as i32,
        _ => 0,
    }
}

fn ele_text(e: Ele) -> String {
    match e {
        Ele::Text(s) => s,
        _ => String::new(),
    }
}

fn opt_pct(v: f32) -> Option<f32> {
    if v < 0.0 { None } else { Some(v) }
}

fn parse_gpu_devices(df: &DataFrame) -> Vec<GpuDeviceRow> {
    let rows = df.cols.first().map(|c| c.len()).unwrap_or(0);
    let idx = |n: &str| col_index(df, n);
    (0..rows)
        .map(|r| {
            let chip = idx("chip")
                .and_then(|c| cell(df, r, c).map(ele_text))
                .filter(|s| !s.trim().is_empty());
            let cc = idx("compute_capability")
                .and_then(|c| cell(df, r, c).map(ele_text))
                .filter(|s| !s.trim().is_empty());
            GpuDeviceRow {
                device_id: idx("device_id")
                    .and_then(|c| cell(df, r, c).map(ele_i32))
                    .unwrap_or(0),
                backend: idx("backend")
                    .and_then(|c| cell(df, r, c).map(ele_text))
                    .unwrap_or_default(),
                name: idx("name")
                    .and_then(|c| cell(df, r, c).map(ele_text))
                    .unwrap_or_default(),
                memory_model: idx("memory_model")
                    .and_then(|c| cell(df, r, c).map(ele_text))
                    .unwrap_or_default(),
                chip,
                compute_capability: cc,
                total_mem_bytes: idx("total_mem_bytes")
                    .and_then(|c| cell(df, r, c).map(ele_i64))
                    .unwrap_or(0),
            }
        })
        .collect()
}

fn parse_gpu_snapshots(df: &DataFrame) -> Vec<GpuSnapshot> {
    let rows = df.cols.first().map(|c| c.len()).unwrap_or(0);
    let idx = |n: &str| col_index(df, n);
    (0..rows)
        .map(|r| {
            let chip = idx("chip")
                .and_then(|c| cell(df, r, c).map(ele_text))
                .filter(|s| !s.trim().is_empty());
            GpuSnapshot {
                ts: idx("ts").and_then(|c| cell(df, r, c).map(ele_i64)).unwrap_or(0),
                device_id: idx("device_id")
                    .and_then(|c| cell(df, r, c).map(ele_i32))
                    .unwrap_or(0),
                backend: idx("backend")
                    .and_then(|c| cell(df, r, c).map(ele_text))
                    .unwrap_or_default(),
                name: idx("name")
                    .and_then(|c| cell(df, r, c).map(ele_text))
                    .unwrap_or_default(),
                memory_model: idx("memory_model")
                    .and_then(|c| cell(df, r, c).map(ele_text))
                    .unwrap_or_default(),
                chip,
                free_bytes: idx("free_bytes")
                    .and_then(|c| cell(df, r, c).map(ele_i64))
                    .unwrap_or(0),
                total_bytes: idx("total_bytes")
                    .and_then(|c| cell(df, r, c).map(ele_i64))
                    .unwrap_or(0),
                used_bytes: idx("used_bytes")
                    .and_then(|c| cell(df, r, c).map(ele_i64))
                    .unwrap_or(0),
                mem_used_pct: idx("mem_used_pct")
                    .and_then(|c| cell(df, r, c).map(ele_f32))
                    .unwrap_or(0.0),
                gpu_util_pct: idx("gpu_util_pct")
                    .and_then(|c| cell(df, r, c).map(ele_f32))
                    .and_then(opt_pct),
                mem_controller_util_pct: idx("mem_controller_util_pct")
                    .and_then(|c| cell(df, r, c).map(ele_f32))
                    .and_then(opt_pct),
                renderer_util_pct: idx("renderer_util_pct")
                    .and_then(|c| cell(df, r, c).map(ele_f32))
                    .and_then(opt_pct),
                tiler_util_pct: idx("tiler_util_pct")
                    .and_then(|c| cell(df, r, c).map(ele_f32))
                    .and_then(opt_pct),
            }
        })
        .collect()
}

fn parse_gpu_history(df: &DataFrame, per_device_limit: usize) -> HashMap<i32, Vec<GpuHistorySample>> {
    let rows = df.cols.first().map(|c| c.len()).unwrap_or(0);
    let idx = |n: &str| col_index(df, n);
    let mut map: HashMap<i32, Vec<GpuHistorySample>> = HashMap::new();

    for r in 0..rows {
        let device_id = idx("device_id")
            .and_then(|c| cell(df, r, c).map(ele_i32))
            .unwrap_or(0);
        let sample = GpuHistorySample {
            mem_used_pct: idx("mem_used_pct")
                .and_then(|c| cell(df, r, c).map(ele_f32))
                .unwrap_or(0.0),
            gpu_util_pct: idx("gpu_util_pct")
                .and_then(|c| cell(df, r, c).map(ele_f32))
                .filter(|&v| v >= 0.0)
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
    map
}

pub fn format_bytes(bytes: i64) -> String {
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    const KB: f64 = 1024.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.1} GB", b / GB)
    } else if b >= MB {
        format!("{:.1} MB", b / MB)
    } else if b >= KB {
        format!("{:.1} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

pub fn gpu_device_label(device_id: i32, name: &str) -> String {
    let short = name.split_whitespace().next().unwrap_or(name);
    if name.len() > 24 {
        format!("GPU {device_id} · {short}…")
    } else {
        format!("GPU {device_id} · {name}")
    }
}

pub fn format_opt_pct(v: Option<f32>) -> String {
    v.map(|p| format!("{p:.1}%"))
        .unwrap_or_else(|| "—".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_missing_gpu_table() {
        let err = AppError::Api("table 'probe.gpu.utilization' not found".into());
        assert!(is_gpu_table_missing(&err));
    }

    #[test]
    fn gpu_device_label_truncates_long_names() {
        let label = gpu_device_label(3, "NVIDIA A100-SXM4-80GB");
        assert!(label.starts_with("GPU 3 ·"));
    }
}
