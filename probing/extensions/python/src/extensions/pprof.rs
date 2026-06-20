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
        _params: &HashMap<String, String>,
        _body: &[u8],
    ) -> Result<Vec<u8>, EngineError> {
        match path.trim_start_matches('/') {
            "flamegraph" => crate::features::pprof::flamegraph()
                .map(|html| html.into_bytes())
                .map_err(|e| EngineError::CallError(e.to_string())),
            "flamegraph/json" => Ok(crate::features::pprof::flamegraph_json().into_bytes()),
            _ => Err(EngineError::UnsupportedCall),
        }
    }
}

impl PprofProbeExtension {
    fn set_sample_freq(&mut self, pprof_sample_freq: Maybe<i32>) -> Result<(), EngineError> {
        // Clearing the option (`set probing.pprof.sample_freq=;`) or a value < 1
        // disables sampling and tears the sampler down.
        let disable = match pprof_sample_freq {
            Maybe::Nothing => true,
            Maybe::Just(freq) => freq < 1,
        };
        if disable {
            crate::features::pprof::reset();
            self.sample_freq = Maybe::Nothing;
            return Ok(());
        }

        let freq = match pprof_sample_freq {
            Maybe::Just(freq) => freq,
            Maybe::Nothing => unreachable!(),
        };
        // Re-settable: `setup` bumps the sampler generation, retires the old
        // consumer thread, and re-arms the timer at the new rate.
        crate::features::pprof::setup(freq as u64).map_err(|e| {
            EngineError::InvalidOptionValue(Self::OPTION_SAMPLE_FREQ.to_string(), e.to_string())
        })?;
        self.sample_freq = pprof_sample_freq.clone();
        Ok(())
    }
}
