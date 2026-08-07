use std::collections::HashMap;

use async_trait::async_trait;
use probing_core::core::EngineError;
use probing_core::core::Maybe;
use probing_core::core::ProbeExtension;
use probing_core::core::ProbeExtensionCall;
use probing_core::core::ProbeExtensionOption;
use probing_core::core::ProbeExtensionResponse;
use pyo3::prelude::*;

#[derive(Debug, Default, ProbeExtension)]
pub struct TorchProbeExtension {
    /// Combined PyTorch profiling specification string (see TorchProbeConfig).
    #[option(aliases=["profiling_mode"])]
    profiling: Maybe<String>,
}

#[async_trait]
impl ProbeExtensionCall for TorchProbeExtension {
    async fn call(
        &self,
        path: &str,
        params: &HashMap<String, String>,
        _body: &[u8],
    ) -> Result<Vec<u8>, EngineError> {
        match path.trim_start_matches('/') {
            "flamegraph" => Ok(crate::features::torch::flamegraph().into_bytes()),
            "flamegraph/json" => {
                let metric = params.get("metric").map(|s| s.as_str());
                Ok(crate::features::torch::flamegraph_json(metric).into_bytes())
            }
            "flamegraph/distributed/json" => {
                Ok(self.distributed_flamegraph_response(params).await?.body)
            }
            _ => Err(EngineError::UnsupportedCall),
        }
    }

    async fn call_response(
        &self,
        path: &str,
        params: &HashMap<String, String>,
        body: &[u8],
    ) -> Result<ProbeExtensionResponse, EngineError> {
        if path.trim_start_matches('/') != "flamegraph/distributed/json" {
            return self.call(path, params, body).await.map(Into::into);
        }

        self.distributed_flamegraph_response(params).await
    }
}

impl TorchProbeExtension {
    async fn distributed_flamegraph_response(
        &self,
        params: &HashMap<String, String>,
    ) -> Result<ProbeExtensionResponse, EngineError> {
        let cluster = super::bool_param(params, "cluster", true)?;
        let step = super::optional_i64_param(params, "step")?;
        let metric = super::one_of_param(
            params,
            "metric",
            "duration",
            &[
                "duration", "delta_mb", "memory", "delta", "mem", "peak_mb", "peak",
            ],
        )?;
        let (body, partial) = crate::features::torch::collect_distributed_flamegraph_json(
            cluster,
            step,
            Some(metric),
        )
        .await?;
        Ok(ProbeExtensionResponse {
            body: body.into_bytes(),
            partial,
        })
    }

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
