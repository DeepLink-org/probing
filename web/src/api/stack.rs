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
            "/apis/training/distributed_stack_flamegraph/json?cluster={cluster}&mode={mode}"
        );
        self.get_request(&path).await
    }
}
