use std::fmt;

/// GPU runtime / vendor backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GpuBackendKind {
    /// NVIDIA CUDA (discrete or datacenter GPU).
    Cuda,
    /// AMD ROCm / HIP (reserved).
    Rocm,
    /// Apple Metal / AGX (M1–M4 integrated GPU).
    Metal,
}

impl GpuBackendKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cuda => "cuda",
            Self::Rocm => "rocm",
            Self::Metal => "metal",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "cuda" | "nvidia" => Some(Self::Cuda),
            "rocm" | "hip" | "amd" => Some(Self::Rocm),
            "metal" | "mps" | "apple" | "agx" | "apple-silicon" => Some(Self::Metal),
            _ => None,
        }
    }
}

impl fmt::Display for GpuBackendKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// How device memory is exposed to the probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuMemoryModel {
    /// Separate VRAM (typical CUDA discrete GPU).
    Dedicated,
    /// CPU/GPU share the same physical memory (Apple Silicon UMA).
    Unified,
}

impl GpuMemoryModel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dedicated => "dedicated",
            Self::Unified => "unified",
        }
    }
}

/// Static device metadata discovered at probe time.
#[derive(Debug, Clone)]
pub struct GpuDeviceInfo {
    pub backend: GpuBackendKind,
    pub ordinal: i32,
    pub name: String,
    pub uuid: Option<String>,
    pub compute_capability: Option<String>,
    pub total_mem_bytes: u64,
    pub memory_model: GpuMemoryModel,
    /// SoC marketing name when available, e.g. "Apple M4 Pro".
    pub chip: Option<String>,
    pub registry_id: Option<u64>,
}

/// One periodic sample for a device (memory + optional utilization).
#[derive(Debug, Clone)]
pub struct GpuMemorySample {
    pub backend: GpuBackendKind,
    pub ordinal: i32,
    pub name: String,
    pub free_bytes: u64,
    pub total_bytes: u64,
    pub memory_model: GpuMemoryModel,
    pub chip: Option<String>,
    /// Overall GPU compute busy % (IORegistry / nvidia-smi).
    pub gpu_util_pct: Option<f32>,
    /// Memory controller busy % (nvidia-smi; distinct from VRAM fill).
    pub mem_controller_util_pct: Option<f32>,
    pub renderer_util_pct: Option<f32>,
    pub tiler_util_pct: Option<f32>,
    /// Unified-memory only: bytes attributed to GPU driver (IORegistry).
    pub driver_mem_bytes: Option<u64>,
}

impl GpuMemorySample {
    pub fn used_bytes(&self) -> u64 {
        self.total_bytes.saturating_sub(self.free_bytes)
    }

    pub fn used_pct(&self) -> f32 {
        if self.total_bytes == 0 {
            return 0.0;
        }
        (self.used_bytes() as f64 / self.total_bytes as f64 * 100.0) as f32
    }
}

/// Backend-specific GPU probe implementation.
pub trait GpuBackend: Send + Sync {
    fn kind(&self) -> GpuBackendKind;

    /// Enumerate devices visible to this backend.
    fn probe_devices(&self) -> Vec<GpuDeviceInfo>;

    /// Sample current device state. Returns `None` if the ordinal is invalid.
    fn sample_memory(&self, ordinal: i32) -> Option<GpuMemorySample>;
}
