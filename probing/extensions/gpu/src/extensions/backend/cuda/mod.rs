mod nvidia_smi;

use std::sync::Arc;

use once_cell::sync::Lazy;

use super::traits::{GpuBackend, GpuBackendKind, GpuDeviceInfo, GpuMemoryModel, GpuMemorySample};
use cudarc::driver::safe::CudaContext;
use cudarc::driver::sys::CUdevice_attribute;

pub use nvidia_smi::read_utilization_by_index;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CudaBackend {
    device_count: i32,
}

/// Cached probe result — cudarc poisons its internal `OnceLock` on failure, so we must
/// never call into cudarc when libcuda / the driver is absent (CI, CPU-only hosts).
static CUDA_BACKEND: Lazy<Option<CudaBackend>> = Lazy::new(probe_cuda_backend);

impl CudaBackend {
    /// Probe for CUDA without panicking when `libcuda` is absent (CI, CPU-only hosts).
    pub fn try_load() -> Option<Self> {
        CUDA_BACKEND.clone()
    }

    pub fn device_count(&self) -> i32 {
        self.device_count
    }

    fn open_context(ordinal: i32) -> Option<Arc<CudaContext>> {
        if ordinal < 0 {
            return None;
        }
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            CudaContext::new(ordinal as usize).ok()
        }))
        .ok()
        .flatten()
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

fn probe_cuda_backend() -> Option<CudaBackend> {
    if !libcuda_driver_ready() {
        log::debug!("CUDA backend unavailable (libcuda or driver not ready)");
        return None;
    }

    let count =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| CudaContext::device_count()))
            .ok()
            .and_then(|r| r.ok())?;

    if count <= 0 {
        log::debug!("CUDA backend unavailable (zero devices)");
        return None;
    }

    log::info!("CUDA backend available ({count} device(s))");
    Some(CudaBackend {
        device_count: count,
    })
}

/// Probe driver via dlopen + `cuInit` — avoids cudarc's fatal `OnceLock` init when no driver exists.
#[cfg(target_os = "linux")]
fn libcuda_driver_ready() -> bool {
    use std::ffi::CString;

    const CUDA_SUCCESS: u32 = 0;
    const NAMES: &[&str] = &[
        "libcuda.so.1",
        "libcuda.so",
        "libnvcuda.so.1",
        "libnvcuda.so",
    ];

    unsafe {
        for name in NAMES {
            let Ok(cname) = CString::new(name) else {
                continue;
            };
            let handle = libc::dlopen(cname.as_ptr(), libc::RTLD_LAZY);
            if handle.is_null() {
                continue;
            }

            let sym = CString::new("cuInit").expect("cuInit");
            let cu_init_ptr = libc::dlsym(handle, sym.as_ptr());
            if cu_init_ptr.is_null() {
                libc::dlclose(handle);
                continue;
            }

            type CuInitFn = unsafe extern "C" fn(u32) -> u32;
            let cu_init: CuInitFn = std::mem::transmute(cu_init_ptr);
            let status = cu_init(0);
            libc::dlclose(handle);

            if status == CUDA_SUCCESS {
                return true;
            }
            log::debug!("cuInit via {name} returned {status}, trying next candidate");
        }
    }
    false
}

#[cfg(not(target_os = "linux"))]
fn libcuda_driver_ready() -> bool {
    // Non-Linux CUDA builds are uncommon; defer to catch_unwind around cudarc.
    true
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
        // Must not panic on repeated probe (CI imports _core many times per process).
        assert_eq!(backend, CudaBackend::try_load());
        if let Some(b) = backend {
            assert!(b.device_count() > 0);
            let devices = b.probe_devices();
            assert_eq!(devices.len() as i32, b.device_count());
        }
    }
}
