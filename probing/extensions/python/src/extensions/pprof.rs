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

impl ProbeExtensionCall for PprofProbeExtension {}

impl PprofProbeExtension {
    fn set_sample_freq(&mut self, pprof_sample_freq: Maybe<i32>) -> Result<(), EngineError> {
        match pprof_sample_freq {
            Maybe::Nothing => Err(EngineError::InvalidOptionValue(
                Self::OPTION_SAMPLE_FREQ.to_string(),
                pprof_sample_freq.clone().into(),
            )),
            Maybe::Just(freq) => {
                if freq < 1 {
                    return Err(EngineError::InvalidOptionValue(
                        Self::OPTION_SAMPLE_FREQ.to_string(),
                        pprof_sample_freq.clone().into(),
                    ));
                }
                // Re-settable: `setup` bumps the sampler generation, retires the
                // old consumer thread, and re-arms the timer at the new rate.
                crate::features::pprof::setup(freq as u64).map_err(|e| {
                    EngineError::InvalidOptionValue(
                        Self::OPTION_SAMPLE_FREQ.to_string(),
                        e.to_string(),
                    )
                })?;
                self.sample_freq = pprof_sample_freq.clone();
                Ok(())
            }
        }
    }
}
