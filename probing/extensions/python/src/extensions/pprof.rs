use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use probing_core::core::federation::FanoutService;
use probing_core::core::EngineError;
use probing_core::core::Maybe;
use probing_core::core::ProbeExtension;
use probing_core::core::ProbeExtensionCall;
use probing_core::core::ProbeExtensionOption;
use probing_core::core::ProbeExtensionResponse;
use probing_core::core::{
    ExtensionConfigSpec, ExtensionContentType, ExtensionHttpMethod, ExtensionRoute,
    ProbeExtensionConfig,
};

#[derive(Debug, ProbeExtension)]
pub struct PprofProbeExtension {
    /// CPU profiling sample frequency in Hz (higher values increase overhead)
    #[option(aliases=["sample.freq"])]
    sample_freq: Maybe<i32>,
    fanout_service: Option<Arc<dyn FanoutService>>,
}

impl Default for PprofProbeExtension {
    fn default() -> Self {
        Self {
            sample_freq: Maybe::Nothing,
            fanout_service: None,
        }
    }
}

#[async_trait]
impl ProbeExtensionCall for PprofProbeExtension {
    fn routes(&self) -> Vec<ExtensionRoute> {
        vec![
            ExtensionRoute::new(
                "flamegraph",
                ExtensionHttpMethod::Get,
                ExtensionContentType::Html,
            ),
            ExtensionRoute::new(
                "flamegraph/json",
                ExtensionHttpMethod::Get,
                ExtensionContentType::Json,
            ),
            ExtensionRoute::new(
                "flamegraph/folded/json",
                ExtensionHttpMethod::Get,
                ExtensionContentType::Json,
            ),
            ExtensionRoute::new(
                "flamegraph/distributed/json",
                ExtensionHttpMethod::Get,
                ExtensionContentType::Json,
            ),
        ]
    }

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

impl PprofProbeExtension {
    pub fn with_fanout_service(fanout_service: Arc<dyn FanoutService>) -> Self {
        Self {
            fanout_service: Some(fanout_service),
            ..Self::default()
        }
    }

    async fn distributed_flamegraph_response(
        &self,
        params: &HashMap<String, String>,
    ) -> Result<ProbeExtensionResponse, EngineError> {
        let cluster = super::bool_param(params, "cluster", true)?;
        let mode = super::one_of_param(params, "mode", "mixed", &["mixed", "py"])?;
        let (body, partial) =
            crate::features::stacktrace::tracers::pprof::collect_distributed_stack_flamegraph_json(
                self.fanout_service.clone(),
                cluster,
                mode,
            )
            .await;
        Ok(ProbeExtensionResponse {
            body: body.into_bytes(),
            partial,
        })
    }

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
