use super::ApiClient;
use crate::utils::error::Result;
use serde::{Deserialize, Serialize};

/// Magic quick action item for UI
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MagicItem {
    pub label: String,
    pub command: String,
    #[serde(default)]
    pub help: String,
}

/// Magic group for UI dropdown
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MagicGroup {
    pub group: String,
    pub items: Vec<MagicItem>,
}

/// Eval execution result (matches Python ExecutionResult / debug_console.push output)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalResponse {
    pub status: String,
    #[serde(default)]
    pub output: String,
    #[serde(default)]
    pub traceback: Vec<String>,
}

/// REPL / Magic command API
impl ApiClient {
    /// Get magic commands as structured list for UI quick actions.
    pub async fn get_magics(&self) -> Result<Vec<MagicGroup>> {
        let path = "/apis/pythonext/magics";
        let text = self.get_request(path).await?;
        serde_json::from_str(&text).map_err(|e| {
            crate::utils::error::AppError::Api(format!("JSON parse error: {e}"))
        })
    }

    /// Execute Python code or magic command in the target process REPL.
    /// Returns the execution result (output, status, traceback).
    pub async fn eval(&self, code: &str) -> Result<EvalResponse> {
        let text = self
            .post_request_with_body("/apis/pythonext/eval", code.to_string())
            .await?;

        // Response may be JSON (from REPL) or plain text (e.g. panic recovery)
        if let Ok(json) = serde_json::from_str::<EvalResponse>(&text) {
            return Ok(json);
        }

        // Fallback: treat as output
        Ok(EvalResponse {
            status: "ok".to_string(),
            output: text,
            traceback: vec![],
        })
    }
}
