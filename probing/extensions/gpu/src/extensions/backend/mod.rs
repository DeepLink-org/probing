mod registry;
mod traits;

#[cfg(target_os = "macos")]
mod apple;

#[cfg(feature = "cuda")]
mod cuda;

pub use registry::{discover_backends, selected_backends};
pub use traits::{GpuBackend, GpuBackendKind, GpuDeviceInfo, GpuMemoryModel, GpuMemorySample};
