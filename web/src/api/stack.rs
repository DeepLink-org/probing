use super::ApiClient;
use crate::utils::error::Result;
use probing_proto::prelude::*;

/// Activity analysis API
impl ApiClient {
    /// Get call stack with mode: mode = py | cpp | mixed
    pub async fn get_callstack_with_mode(
        &self,
        tid: Option<String>,
        mode: &str,
    ) -> Result<Vec<CallFrame>> {
        let mode = match mode {
            "py" | "cpp" | "mixed" => mode,
            _ => "mixed",
        };
        let base = "/apis/pythonext/callstack";
        let path = if let Some(tid) = tid {
            format!("{}?tid={}&mode={}", base, tid, mode)
        } else {
            format!("{}?mode={}", base, mode)
        };
        let response = self.get_request(&path).await?;
        Self::parse_json(&response)
    }

    /// Distributed CPU stack flamegraph (`mode`: `mixed` | `py`).
    pub async fn get_distributed_stack_flamegraph_json(
        &self,
        cluster: bool,
        mode: &str,
    ) -> Result<String> {
        let mode = match mode {
            "py" | "mixed" => mode,
            _ => "mixed",
        };
        let path = format!(
            "/apis/pprofextension/flamegraph/distributed/json?cluster={cluster}&mode={mode}"
        );
        let body = self
            .get_request_accepting_with_timeout(
                &path,
                &[503],
                Some(std::time::Duration::from_secs(20)),
            )
            .await?;
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&body) {
            if value.get("profile").is_none() {
                if let Some(error) = value.get("error").and_then(|error| error.as_str()) {
                    return Err(crate::utils::error::AppError::Api(error.to_string()));
                }
            }
        }
        Ok(body)
    }
}
