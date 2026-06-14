mod nvidia_smi;

use std::sync::Arc;

use super::traits::{
    GpuBackend, GpuBackendKind, GpuDeviceInfo, GpuMemoryModel, GpuMemorySample,
};
use cudarc::driver::safe::CudaContext;
use cudarc::driver::sys::CUdevice_attribute;

pub use nvidia_smi::{read_utilization_by_index, NvidiaDeviceStats};

pub struct CudaBackend {
    device_count: i32,
}

impl CudaBackend {
    pub fn try_load() -> Option<Self> {
        let count = CudaContext::device_count().ok()?;
        if count <= 0 {
            return None;
        }
        Some(Self { device_count: count })
    }

    pub fn device_count(&self) -> i32 {
        self.device_count
    }

    fn open_context(ordinal: i32) -> Option<Arc<CudaContext>> {
        if ordinal < 0 {
            return None;
        }
        CudaContext::new(ordinal as usize).ok()
    }

    fn device_info(ctx: &CudaContext, ordinal: i32) -> GpuDeviceInfo {
        let name = ctx.name().unwrap_or_else(|_| format!("cuda:{ordinal}"));
        let uuid = ctx.uuid().ok().map(format_cuda_uuid);
        let compute_capability = compute_capability(ctx);
        let total_mem_bytes = ctx.total_mem().unwrap_or(0) as u64;

        GpuDeviceInfo {
            backend: GpuBackendKind::Cuda,
            ordinal,
            name,
            uuid,
            compute_capability,
            total_mem_bytes,
            memory_model: GpuMemoryModel::Dedicated,
            chip: None,
            registry_id: None,
        }
    }
}

impl GpuBackend for CudaBackend {
    fn kind(&self) -> GpuBackendKind {
        GpuBackendKind::Cuda
    }

    fn probe_devices(&self) -> Vec<GpuDeviceInfo> {
        (0..self.device_count)
            .filter_map(|ordinal| {
                Self::open_context(ordinal).map(|ctx| Self::device_info(&ctx, ordinal))
            })
            .collect()
    }

    fn sample_memory(&self, ordinal: i32) -> Option<GpuMemorySample> {
        if ordinal < 0 || ordinal >= self.device_count {
            return None;
        }
        let ctx = Self::open_context(ordinal)?;
        let name = ctx.name().unwrap_or_else(|_| format!("cuda:{ordinal}"));
        let (free, total) = ctx.mem_get_info().ok()?;
        Some(GpuMemorySample {
            backend: self.kind(),
            ordinal,
            name,
            free_bytes: free as u64,
            total_bytes: total as u64,
            memory_model: GpuMemoryModel::Dedicated,
            chip: None,
            gpu_util_pct: None,
            mem_controller_util_pct: None,
            renderer_util_pct: None,
            tiler_util_pct: None,
            driver_mem_bytes: None,
        })
    }
}

fn compute_capability(ctx: &CudaContext) -> Option<String> {
    let major = ctx
        .attribute(CUdevice_attribute::CU_DEVICE_ATTRIBUTE_COMPUTE_CAPABILITY_MAJOR)
        .ok()?;
    let minor = ctx
        .attribute(CUdevice_attribute::CU_DEVICE_ATTRIBUTE_COMPUTE_CAPABILITY_MINOR)
        .ok()?;
    Some(format!("{major}.{minor}"))
}

fn format_cuda_uuid(uuid: cudarc::driver::sys::CUuuid) -> String {
    uuid.bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cuda_backend_loads_or_skips_gracefully() {
        let backend = CudaBackend::try_load();
        if let Some(b) = backend {
            assert!(b.device_count() > 0);
            let devices = b.probe_devices();
            assert_eq!(devices.len() as i32, b.device_count());
        }
    }
}
