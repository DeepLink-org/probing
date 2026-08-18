mod registry;
mod traits;

#[cfg(target_os = "macos")]
mod apple;

#[cfg(feature = "cuda")]
mod cuda;

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
mod npu;

pub use registry::{discover_backends, selected_backends};
pub use traits::{GpuBackend, GpuBackendKind, GpuDeviceInfo, GpuMemoryModel, GpuMemorySample};

#[cfg(feature = "cuda")]
pub use cuda::read_utilization_by_index;

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
pub use npu::read_utilization_by_index as read_npu_utilization_by_index;
