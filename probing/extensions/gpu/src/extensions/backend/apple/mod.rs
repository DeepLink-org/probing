mod ioreg;
mod sysctl;

use metal::Device;

use super::traits::{GpuBackend, GpuBackendKind, GpuDeviceInfo, GpuMemoryModel, GpuMemorySample};

pub struct AppleSiliconBackend {
    devices: Vec<AppleDevice>,
    chip: Option<String>,
    system_mem_bytes: u64,
}

struct AppleDevice {
    ordinal: i32,
    device: Device,
    name: String,
    registry_id: u64,
    unified: bool,
}

impl AppleSiliconBackend {
    pub fn try_load() -> Option<Self> {
        let mut metal_devices = Device::all();
        if metal_devices.is_empty() {
            if let Some(default) = Device::system_default() {
                metal_devices.push(default);
            }
        }
        Self::from_devices(metal_devices)
    }

    fn from_devices(metal_devices: Vec<Device>) -> Option<Self> {
        if metal_devices.is_empty() {
            return None;
        }

        let chip = sysctl::cpu_brand_string();
        let system_mem_bytes = sysctl::hw_memsize().unwrap_or(0);
        let mut devices = Vec::with_capacity(metal_devices.len());

        for (ordinal, device) in metal_devices.into_iter().enumerate() {
            devices.push(AppleDevice {
                ordinal: ordinal as i32,
                name: device.name().to_string(),
                registry_id: device.registry_id(),
                unified: device.has_unified_memory(),
                device,
            });
        }

        Some(Self {
            devices,
            chip,
            system_mem_bytes,
        })
    }

    fn device_info(&self, dev: &AppleDevice) -> GpuDeviceInfo {
        let budget = dev.device.recommended_max_working_set_size();
        let total = if budget > 0 {
            budget
        } else {
            self.system_mem_bytes
        };

        GpuDeviceInfo {
            backend: GpuBackendKind::Metal,
            ordinal: dev.ordinal,
            name: dev.name.clone(),
            uuid: None,
            compute_capability: None,
            total_mem_bytes: total,
            memory_model: if dev.unified {
                GpuMemoryModel::Unified
            } else {
                GpuMemoryModel::Dedicated
            },
            chip: self.chip.clone(),
            registry_id: Some(dev.registry_id),
        }
    }
}

impl GpuBackend for AppleSiliconBackend {
    fn kind(&self) -> GpuBackendKind {
        GpuBackendKind::Metal
    }

    fn probe_devices(&self) -> Vec<GpuDeviceInfo> {
        self.devices
            .iter()
            .map(|dev| self.device_info(dev))
            .collect()
    }

    fn sample_memory(&self, ordinal: i32) -> Option<GpuMemorySample> {
        let dev = self.devices.iter().find(|d| d.ordinal == ordinal)?;
        let perf = ioreg::read_performance_stats();
        let allocated = dev.device.current_allocated_size() as u64;
        let budget = dev.device.recommended_max_working_set_size();
        let total = if budget > 0 {
            budget
        } else {
            self.system_mem_bytes
        };

        let (free_bytes, driver_mem_bytes) = if let Some(ref stats) = perf {
            let in_use = stats.in_use_system_memory.unwrap_or(allocated);
            (
                total.saturating_sub(in_use),
                stats.in_use_system_memory.or(Some(allocated)),
            )
        } else {
            (total.saturating_sub(allocated), Some(allocated))
        };

        Some(GpuMemorySample {
            backend: GpuBackendKind::Metal,
            ordinal: dev.ordinal,
            name: dev.name.clone(),
            free_bytes,
            total_bytes: total,
            memory_model: if dev.unified {
                GpuMemoryModel::Unified
            } else {
                GpuMemoryModel::Dedicated
            },
            chip: self.chip.clone(),
            gpu_util_pct: perf.as_ref().and_then(|p| p.device_util_pct),
            mem_controller_util_pct: None,
            renderer_util_pct: perf.as_ref().and_then(|p| p.renderer_util_pct),
            tiler_util_pct: perf.as_ref().and_then(|p| p.tiler_util_pct),
            driver_mem_bytes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apple_backend_loads_on_host() {
        let backend = AppleSiliconBackend::try_load();
        if let Some(b) = backend {
            let devices = b.probe_devices();
            assert!(!devices.is_empty(), "expected at least one Metal device");
            assert_eq!(devices[0].memory_model, GpuMemoryModel::Unified);
            assert!(devices[0].chip.as_deref().unwrap_or("").contains("Apple"));
        }
    }
}
