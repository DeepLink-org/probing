use probing_core::core::EngineError;
use probing_core::core::Maybe;
use probing_core::core::ProbeExtension;
use probing_core::core::ProbeExtensionCall;
use probing_core::core::ProbeExtensionOption;

use super::collector::start_gpu_sampling;

#[derive(Debug, Default, ProbeExtension)]
pub struct GpuProbeExtension {
    /// GPU memory sampling interval in milliseconds (0 disables collection).
    #[option(aliases = ["sample_interval", "interval", "gpu.interval"])]
    gpu_sample_interval_ms: Maybe<i64>,

    /// Backend filter: `auto`, `cuda`, `rocm`, `metal`, or comma-separated list.
    #[option(aliases = ["backend", "gpu.backend"])]
    gpu_backend: Maybe<String>,
}

impl ProbeExtensionCall for GpuProbeExtension {}

impl GpuProbeExtension {
    fn set_gpu_sample_interval_ms(
        &mut self,
        gpu_sample_interval_ms: Maybe<i64>,
    ) -> Result<(), EngineError> {
        let Maybe::Just(interval) = gpu_sample_interval_ms.clone() else {
            return Err(EngineError::InvalidOptionValue(
                Self::OPTION_GPU_SAMPLE_INTERVAL_MS.to_string(),
                gpu_sample_interval_ms.clone().into(),
            ));
        };

        if interval < 0 {
            return Err(EngineError::InvalidOptionValue(
                Self::OPTION_GPU_SAMPLE_INTERVAL_MS.to_string(),
                gpu_sample_interval_ms.clone().into(),
            ));
        }

        if interval == 0 {
            self.gpu_sample_interval_ms = gpu_sample_interval_ms;
            return Ok(());
        }

        if let Maybe::Just(current) = self.gpu_sample_interval_ms {
            if current == interval {
                return Ok(());
            }
            return Err(EngineError::InvalidOptionValue(
                Self::OPTION_GPU_SAMPLE_INTERVAL_MS.to_string(),
                gpu_sample_interval_ms.clone().into(),
            ));
        }

        if let Maybe::Just(ref backend) = self.gpu_backend {
            std::env::set_var("PROBING_GPU_BACKEND", backend);
        }

        start_gpu_sampling(interval as u64).map_err(|e| {
            EngineError::InvalidOptionValue(
                Self::OPTION_GPU_SAMPLE_INTERVAL_MS.to_string(),
                format!("{e}").into(),
            )
        })?;

        self.gpu_sample_interval_ms = gpu_sample_interval_ms;
        Ok(())
    }

    fn set_gpu_backend(&mut self, gpu_backend: Maybe<String>) -> Result<(), EngineError> {
        if matches!(self.gpu_backend, Maybe::Just(_)) {
            return Err(EngineError::InvalidOptionValue(
                Self::OPTION_GPU_BACKEND.to_string(),
                gpu_backend.clone().into(),
            ));
        }

        match gpu_backend {
            Maybe::Just(ref value) if !value.trim().is_empty() => {
                std::env::set_var("PROBING_GPU_BACKEND", value.trim());
                self.gpu_backend = gpu_backend;
                Ok(())
            }
            _ => Err(EngineError::InvalidOptionValue(
                Self::OPTION_GPU_BACKEND.to_string(),
                gpu_backend.into(),
            )),
        }
    }
}
