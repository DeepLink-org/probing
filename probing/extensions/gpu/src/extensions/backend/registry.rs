use std::sync::Mutex;

use once_cell::sync::Lazy;

use super::traits::{GpuBackend, GpuBackendKind};

#[cfg(target_os = "macos")]
use super::apple::AppleSiliconBackend;

#[cfg(feature = "cuda")]
use super::cuda::CudaBackend;

static BACKEND_FILTER: Lazy<Mutex<Option<Vec<GpuBackendKind>>>> = Lazy::new(|| Mutex::new(None));

/// Restrict which backends are active (`None` = auto-discover all available).
pub fn set_backend_filter(kinds: Option<Vec<GpuBackendKind>>) {
    *BACKEND_FILTER.lock().unwrap() = kinds;
}

#[cfg_attr(not(any(feature = "cuda", target_os = "macos")), allow(dead_code))]
fn filter_allows(kind: GpuBackendKind) -> bool {
    match BACKEND_FILTER.lock().unwrap().as_ref() {
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
                if let Some(cuda) = CudaBackend::try_load() {
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

    #[cfg(not(any(feature = "cuda", target_os = "macos")))]
    log::debug!("probing-gpu: no GPU backends available for this target");

    [cuda, apple].into_iter().flatten().collect()
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

    /// Linux/Windows CI: crate must compile with zero GPU backends when cuda is off.
    #[test]
    #[cfg(all(not(target_os = "macos"), not(feature = "cuda")))]
    fn non_mac_build_has_no_backends_without_cuda() {
        assert!(discover_backends().is_empty());
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
