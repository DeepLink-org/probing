use std::collections::HashMap;

use async_trait::async_trait;
use probing_core::core::EngineError;
use probing_core::core::Maybe;
use probing_core::core::ProbeExtension;
use probing_core::core::ProbeExtensionCall;
use probing_core::core::ProbeExtensionOption;

#[derive(Debug, Default, ProbeExtension)]
pub struct PprofProbeExtension {
    /// CPU profiling sample frequency in Hz (higher values increase overhead)
    #[option(aliases=["sample.freq"])]
    sample_freq: Maybe<i32>,
}

#[async_trait]
impl ProbeExtensionCall for PprofProbeExtension {
    async fn call(
        &self,
        path: &str,
        params: &HashMap<String, String>,
        _body: &[u8],
    ) -> Result<Vec<u8>, EngineError> {
        match path.trim_start_matches('/') {
            "flamegraph" => crate::features::stacktrace::tracers::pprof::flamegraph()
                .map(|html| html.into_bytes())
                .map_err(|e| EngineError::CallError(e.to_string())),
            "flamegraph/json" => {
                Ok(crate::features::stacktrace::tracers::pprof::flamegraph_json().into_bytes())
            }
            "flamegraph/folded/json" => {
                Ok(crate::features::stacktrace::tracers::pprof::folded_lines_json().into_bytes())
            }
            "flamegraph/distributed/json" => {
                let cluster = params
                    .get("cluster")
                    .map(|v| v.as_str())
                    .map(|v| v != "0" && v != "false")
                    .unwrap_or(true);
                let mode = params.get("mode").map(|s| s.as_str()).unwrap_or("mixed");
                let (body, _) = crate::features::stacktrace::tracers::pprof::collect_distributed_stack_flamegraph_json(
                    cluster, mode,
                )
                .await;
                Ok(body.into_bytes())
            }
            _ => Err(EngineError::UnsupportedCall),
        }
    }
}

impl PprofProbeExtension {
    fn set_sample_freq(&mut self, pprof_sample_freq: Maybe<i32>) -> Result<(), EngineError> {
        // Clearing the option (`set probing.pprof.sample_freq=;`) or a value < 1
        // disables sampling and tears the sampler down.
        let freq = match pprof_sample_freq {
            Maybe::Just(freq) if freq >= 1 => freq,
            _ => {
                crate::features::stacktrace::tracers::pprof::reset();
                self.sample_freq = Maybe::Nothing;
                return Ok(());
            }
        };
        // Re-settable: `setup` bumps the sampler generation, retires the old
        // consumer thread, and re-arms the timer at the new rate.
        crate::features::stacktrace::tracers::pprof::setup(freq as u64).map_err(|e| {
            EngineError::InvalidOptionValue(Self::OPTION_SAMPLE_FREQ.to_string(), e.to_string())
        })?;
        self.sample_freq = pprof_sample_freq.clone();
        Ok(())
    }
}
