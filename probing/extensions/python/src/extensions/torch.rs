use probing_core::core::ProbeExtensionCall;
use probing_core::core::EngineError;
use probing_core::core::ProbeExtension;
use probing_core::core::ProbeExtensionOption;
use probing_core::core::Maybe;
use pyo3::prelude::*;

#[derive(Debug, Default, ProbeExtension)]
pub struct TorchProbeExtension {
    /// Combined PyTorch profiling specification string (see TorchProbeConfig).
    #[option(aliases=["profiling_mode"])]
    profiling: Maybe<String>,
}

impl ProbeExtensionCall for TorchProbeExtension {}

impl TorchProbeExtension {
    fn set_profiling(&mut self, profiling: Maybe<String>) -> Result<(), EngineError> {
        let py_result = Python::attach(|py| -> pyo3::PyResult<()> {
            let module = py.import("probing.profiling.torch_probe")?;
            match &profiling {
                Maybe::Just(spec) => {
                    if spec.trim().is_empty() {
                        module.call_method1("configure", (Option::<&str>::None,))?;
                    } else {
                        module.call_method1("configure", (spec.as_str(),))?;
                    }
                }
                Maybe::Nothing => {
                    module.call_method1("configure", (Option::<&str>::None,))?;
                }
            }
            Ok(())
        });

        match py_result {
            Ok(()) => {
                self.profiling = profiling;
                Ok(())
            }
            Err(err) => {
                let value: String = profiling.clone().into();
                log::error!(
                    "Failed to configure torch profiling with spec '{}': {}",
                    value,
                    err
                );
                Err(EngineError::InvalidOptionValue(
                    Self::OPTION_PROFILING.to_string(),
                    value,
                ))
            }
        }
    }
}
