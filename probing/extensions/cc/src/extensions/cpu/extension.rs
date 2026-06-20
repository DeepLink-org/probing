use probing_core::core::EngineError;
use probing_core::core::Maybe;
use probing_core::core::ProbeExtension;
use probing_core::core::ProbeExtensionCall;
use probing_core::core::ProbeExtensionOption;

use super::collector::start_cpu_sampling;

#[derive(Debug, Default, ProbeExtension)]
pub struct CpuProbeExtension {
    /// CPU sampling interval in milliseconds (0 disables collection).
    #[option(aliases = [
        "sample_interval",
        "interval",
        "taskstats_interval",
        "task_stats_interval",
        "task.stats.interval"
    ])]
    cpu_sample_interval_ms: Maybe<i64>,

    /// Max threads to record per sample (0 = process-level only).
    #[option(aliases = ["thread_top_n"])]
    cpu_thread_top_n: Maybe<i64>,
}

impl ProbeExtensionCall for CpuProbeExtension {}

impl CpuProbeExtension {
    fn set_cpu_sample_interval_ms(
        &mut self,
        cpu_sample_interval_ms: Maybe<i64>,
    ) -> Result<(), EngineError> {
        let Maybe::Just(interval) = cpu_sample_interval_ms.clone() else {
            return Err(EngineError::InvalidOptionValue(
                Self::OPTION_CPU_SAMPLE_INTERVAL_MS.to_string(),
                cpu_sample_interval_ms.clone().into(),
            ));
        };

        if interval < 0 {
            return Err(EngineError::InvalidOptionValue(
                Self::OPTION_CPU_SAMPLE_INTERVAL_MS.to_string(),
                cpu_sample_interval_ms.clone().into(),
            ));
        }

        if interval == 0 {
            self.cpu_sample_interval_ms = cpu_sample_interval_ms;
            return Ok(());
        }

        if let Maybe::Just(current) = self.cpu_sample_interval_ms {
            if current == interval {
                return Ok(());
            }
            return Err(EngineError::InvalidOptionValue(
                Self::OPTION_CPU_SAMPLE_INTERVAL_MS.to_string(),
                cpu_sample_interval_ms.clone().into(),
            ));
        }

        let thread_top_n = match self.cpu_thread_top_n {
            Maybe::Just(n) if n >= 0 => n as usize,
            _ => 8,
        };

        start_cpu_sampling(interval as u64, thread_top_n).map_err(|e| {
            EngineError::InvalidOptionValue(
                Self::OPTION_CPU_SAMPLE_INTERVAL_MS.to_string(),
                format!("{e}"),
            )
        })?;

        self.cpu_sample_interval_ms = cpu_sample_interval_ms;
        Ok(())
    }

    fn set_cpu_thread_top_n(&mut self, cpu_thread_top_n: Maybe<i64>) -> Result<(), EngineError> {
        if matches!(self.cpu_thread_top_n, Maybe::Just(_)) {
            return Err(EngineError::InvalidOptionValue(
                Self::OPTION_CPU_THREAD_TOP_N.to_string(),
                cpu_thread_top_n.clone().into(),
            ));
        }

        match cpu_thread_top_n {
            Maybe::Just(n) if n >= 0 => {
                self.cpu_thread_top_n = cpu_thread_top_n;
                Ok(())
            }
            _ => Err(EngineError::InvalidOptionValue(
                Self::OPTION_CPU_THREAD_TOP_N.to_string(),
                cpu_thread_top_n.into(),
            )),
        }
    }
}
