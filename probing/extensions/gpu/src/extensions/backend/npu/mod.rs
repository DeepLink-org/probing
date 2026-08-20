//! Huawei Ascend NPU backend (DCMI preferred, `npu-smi` fallback).

mod dcmi;
#[allow(dead_code)]
mod hccs;
mod npu_smi;

use std::sync::Arc;

use once_cell::sync::Lazy;

use super::traits::{GpuBackend, GpuBackendKind, GpuDeviceInfo, GpuMemoryModel, GpuMemorySample};

pub use npu_smi::read_utilization_by_index;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NpuBackend {
    device_ids: Arc<Vec<i32>>,
}

static NPU_BACKEND: Lazy<Option<NpuBackend>> = Lazy::new(probe_npu_backend);

impl NpuBackend {
    pub fn try_load() -> Option<Self> {
        NPU_BACKEND.clone()
    }

    pub fn device_count(&self) -> usize {
        self.device_ids.len()
    }
}

impl GpuBackend for NpuBackend {
    fn kind(&self) -> GpuBackendKind {
        GpuBackendKind::Npu
    }

    fn probe_devices(&self) -> Vec<GpuDeviceInfo> {
        self.device_ids
            .iter()
            .map(|&ordinal| {
                let detail = npu_smi::read_device_detail(ordinal);
                let chip = detail.chip_name.clone();
                GpuDeviceInfo {
                    backend: GpuBackendKind::Npu,
                    ordinal,
                    name: chip
                        .clone()
                        .unwrap_or_else(|| format!("ascend-npu:{ordinal}")),
                    uuid: None,
                    compute_capability: None,
                    total_mem_bytes: detail.total_mem_bytes,
                    memory_model: GpuMemoryModel::Dedicated,
                    chip,
                    registry_id: None,
                }
            })
            .collect()
    }

    fn sample_memory(&self, ordinal: i32) -> Option<GpuMemorySample> {
        if !self.device_ids.contains(&ordinal) {
            return None;
        }
        let detail = npu_smi::read_device_detail(ordinal);
        let stats = npu_smi::read_utilization_by_index().get(&ordinal).copied();
        let chip = detail.chip_name.clone();
        let (free_bytes, total_bytes) = if detail.total_mem_bytes > 0 {
            (detail.free_mem_bytes, detail.total_mem_bytes)
        } else if let Some(s) = stats {
            (
                s.hbm_total_bytes.saturating_sub(s.hbm_used_bytes),
                s.hbm_total_bytes,
            )
        } else {
            (0, 0)
        };
        Some(GpuMemorySample {
            backend: self.kind(),
            ordinal,
            name: chip
                .clone()
                .unwrap_or_else(|| format!("ascend-npu:{ordinal}")),
            free_bytes,
            total_bytes,
            memory_model: GpuMemoryModel::Dedicated,
            chip,
            // Also set here so callers that skip collector merge still see util.
            gpu_util_pct: stats.map(|s| s.ai_core_util_pct),
            mem_controller_util_pct: stats.map(|s| s.hbm_util_pct),
            // Preserve compatibility with the existing web snapshot columns.
            renderer_util_pct: stats.and_then(|s| s.aivector_util_pct),
            tiler_util_pct: stats.and_then(|s| s.hbm_bw_util_pct),
            driver_mem_bytes: None,
            vector_util_pct: stats.and_then(|s| s.aivector_util_pct),
            mem_bandwidth_util_pct: stats.and_then(|s| s.hbm_bw_util_pct),
            temperature_c: stats.and_then(|s| s.temp_c),
            power_w: stats.and_then(|s| s.power_w),
        })
    }
}

fn probe_npu_backend() -> Option<NpuBackend> {
    let device_ids = npu_smi::list_device_ids();
    if device_ids.is_empty() {
        log::debug!("NPU backend unavailable (DCMI/npu-smi missing or no devices)");
        return None;
    }
    let source = npu_smi::read_utilization_by_index()
        .values()
        .next()
        .map(|s| s.source)
        .unwrap_or("unknown");
    log::info!(
        "NPU backend probe: {} device(s) via {source}",
        device_ids.len()
    );
    Some(NpuBackend {
        device_ids: Arc::new(device_ids),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn npu_backend_probe_does_not_panic() {
        let _ = NpuBackend::try_load();
    }
}
