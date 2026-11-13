//! In-process NVML sampling: no `nvidia-smi` subprocess, no CUDA contexts.
//!
//! The previous path forked one `nvidia-smi` per sample tick and touched a
//! cudarc `CudaContext` twice per device; both contend on driver locks with
//! concurrent CUDA/NCCL work and measurably slow training running in other
//! processes (+26% step time at 1s intervals, +5.7% at 60s on 8×H200 PPO;
//! in-process NVML at 60s measured ~0%, matching xtuner's pynvml monitor).
//! NVML queries below fill utilization, memory, temperature and power in one
//! pass per device. cudarc remains only for one-time device enumeration.

use std::collections::HashMap;
use std::ffi::OsStr;

use nvml_wrapper::enum_wrappers::device::TemperatureSensor;
use nvml_wrapper::Nvml;
use once_cell::sync::Lazy;

/// Per-device stats read in-process via NVML.
#[derive(Debug, Clone)]
pub struct NvmlSample {
    pub name: String,
    pub gpu_util_pct: f32,
    pub mem_controller_util_pct: f32,
    pub free_bytes: u64,
    pub total_bytes: u64,
    #[allow(dead_code)] // parity with nvidia-smi output; used_pct derives from free/total
    pub used_bytes: u64,
    pub temperature_c: f32,
    pub power_w: f32,
}

static NVML: Lazy<Option<Nvml>> = Lazy::new(init_nvml);

/// nvml-wrapper dlopens ``libnvidia-ml.so`` by default, but datacenter driver
/// installs commonly ship only the versioned ``libnvidia-ml.so.1``; try the
/// default first, then versioned/absolute candidates before giving up.
fn init_nvml() -> Option<Nvml> {
    if let Ok(nvml) = Nvml::init() {
        log::debug!("NVML initialized for in-process GPU sampling");
        return Some(nvml);
    }
    for candidate in [
        "libnvidia-ml.so.1",
        "/lib/x86_64-linux-gnu/libnvidia-ml.so.1",
        "/usr/lib64/libnvidia-ml.so.1",
    ] {
        match Nvml::builder().lib_path(OsStr::new(candidate)).init() {
            Ok(nvml) => {
                log::debug!("NVML initialized for in-process GPU sampling via {candidate}");
                return Some(nvml);
            }
            Err(e) => log::debug!("NVML candidate {candidate} failed: {e}"),
        }
    }
    log::debug!("NVML unavailable, GPU sampling falls back to legacy path");
    None
}

/// Read utilization + memory + temperature + power for every device in one pass.
pub fn sample_all_devices() -> HashMap<i32, NvmlSample> {
    let mut out = HashMap::new();
    let Some(nvml) = NVML.as_ref() else {
        return out;
    };
    let Ok(count) = nvml.device_count() else {
        return out;
    };
    for index in 0..count {
        let Ok(device) = nvml.device_by_index(index) else {
            continue;
        };
        let util = device.utilization_rates().ok();
        let (free, total, used) = device
            .memory_info()
            .map(|m| (m.free, m.total, m.used))
            .unwrap_or((0, 0, 0));
        out.insert(
            index as i32,
            NvmlSample {
                name: device.name().unwrap_or_else(|_| format!("nvml:{index}")),
                gpu_util_pct: util.as_ref().map(|u| u.gpu as f32).unwrap_or(-1.0),
                mem_controller_util_pct: util.as_ref().map(|u| u.memory as f32).unwrap_or(-1.0),
                free_bytes: free,
                total_bytes: total,
                used_bytes: used,
                temperature_c: device
                    .temperature(TemperatureSensor::Gpu)
                    .map(|t| t as f32)
                    .unwrap_or(-1.0),
                power_w: device
                    .power_usage()
                    .map(|milliwatts| milliwatts as f32 / 1000.0)
                    .unwrap_or(-1.0),
            },
        );
    }
    out
}
