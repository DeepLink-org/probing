use std::sync::Mutex;

use once_cell::sync::Lazy;
use probing_core::sync::lock_mutex;

use super::traits::{GpuBackend, GpuBackendKind};

#[cfg(target_os = "macos")]
use super::apple::AppleSiliconBackend;

#[cfg(feature = "cuda")]
use super::cuda::CudaBackend;

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
use super::npu::NpuBackend;

static BACKEND_FILTER: Lazy<Mutex<Option<Vec<GpuBackendKind>>>> = Lazy::new(|| Mutex::new(None));

/// Restrict which backends are active (`None` = auto-discover all available).
pub fn set_backend_filter(kinds: Option<Vec<GpuBackendKind>>) {
    *lock_mutex(&BACKEND_FILTER, "GPU backend filter") = kinds;
}

#[cfg_attr(
    not(any(
        feature = "cuda",
        target_os = "macos",
        all(not(target_os = "macos"), not(target_os = "windows"))
    )),
    allow(dead_code)
)]
fn filter_allows(kind: GpuBackendKind) -> bool {
    match lock_mutex(&BACKEND_FILTER, "GPU backend filter").as_ref() {
        None => true,
        Some(list) => list.contains(&kind),
    }
}

/// Discover all GPU backends available on this host (dlopen / runtime probe; never panics).
pub fn discover_backends() -> Vec<Box<dyn GpuBackend>> {
    let cuda: Option<Box<dyn GpuBackend>> = {
        #[cfg(feature = "cuda")]
        {
            if filter_allows(GpuBackendKind::Cuda) {
                let cuda =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(CudaBackend::try_load))
                        .ok()
                        .flatten();
                if let Some(cuda) = cuda {
                    log::info!(
                        "GPU backend loaded: cuda ({} device(s))",
                        cuda.device_count()
                    );
                    Some(Box::new(cuda))
                } else {
                    log::debug!("CUDA backend unavailable (no driver or libcuda)");
                    None
                }
            } else {
                None
            }
        }
        #[cfg(not(feature = "cuda"))]
        {
            None
        }
    };

    let apple: Option<Box<dyn GpuBackend>> = {
        #[cfg(target_os = "macos")]
        {
            if filter_allows(GpuBackendKind::Metal) {
                if let Some(apple) = AppleSiliconBackend::try_load() {
                    let count = apple.probe_devices().len();
                    log::info!("GPU backend loaded: metal/apple-silicon ({count} device(s))");
                    Some(Box::new(apple))
                } else {
                    log::debug!("Apple Silicon GPU backend unavailable (no Metal device)");
                    None
                }
            } else {
                None
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            None
        }
    };

    let npu: Option<Box<dyn GpuBackend>> = {
        #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
        {
            if filter_allows(GpuBackendKind::Npu) {
                let npu =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(NpuBackend::try_load))
                        .ok()
                        .flatten();
                if let Some(npu) = npu {
                    let count = npu.device_count();
                    log::info!("GPU backend loaded: ascend/npu ({count} device(s))");
                    Some(Box::new(npu))
                } else {
                    log::debug!("Ascend NPU backend unavailable (DCMI/npu-smi missing)");
                    None
                }
            } else {
                None
            }
        }
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            None
        }
    };

    #[cfg(not(any(
        feature = "cuda",
        target_os = "macos",
        all(not(target_os = "macos"), not(target_os = "windows"))
    )))]
    log::debug!("probing-gpu: no GPU backends available for this target");

    [cuda, apple, npu].into_iter().flatten().collect()
}

/// Backends selected by env `PROBING_GPU_BACKEND` (default: auto = all discovered).
pub fn selected_backends() -> Vec<Box<dyn GpuBackend>> {
    let filter = std::env::var("PROBING_GPU_BACKEND").ok().and_then(|raw| {
        let trimmed = raw.trim().to_ascii_lowercase();
        if matches!(trimmed.as_str(), "" | "auto" | "all" | "any") {
            return None;
        }
        Some(
            trimmed
                .split([',', ' ', ';'])
                .filter_map(GpuBackendKind::parse)
                .collect::<Vec<_>>(),
        )
    });

    set_backend_filter(filter);
    discover_backends()
}

#[cfg(test)]
mod platform_tests {
    use super::*;

    /// Linux/Windows CI: discovery must never abort when libcuda is missing.
    #[test]
    #[cfg(feature = "cuda")]
    fn discover_backends_is_idempotent_without_cuda() {
        let first = discover_backends();
        let second = discover_backends();
        assert_eq!(first.len(), second.len());
    }

    /// A no-CUDA build must not discover CUDA; platform-native backends may remain.
    #[test]
    #[cfg(all(not(target_os = "macos"), not(feature = "cuda")))]
    fn non_mac_build_has_no_cuda_backend_without_cuda() {
        assert!(!discover_backends()
            .iter()
            .any(|backend| backend.kind() == GpuBackendKind::Cuda));
    }

    /// macOS: Apple backend module is linked; discovery may return devices.
    #[test]
    #[cfg(target_os = "macos")]
    fn macos_discovers_apple_backend_when_available() {
        use super::super::apple::AppleSiliconBackend;

        let backends = discover_backends();
        if AppleSiliconBackend::try_load().is_some() {
            assert!(!backends.is_empty());
            assert!(backends.iter().any(|b| b.kind() == GpuBackendKind::Metal));
        }
    }
}
