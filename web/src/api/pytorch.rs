use super::ApiClient;
use crate::utils::error::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProfileResponse {
    pub success: bool,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeDebugResponse {
    pub wait_counters: WaitCounterSnapshot,
    pub tcpstore: TcpStoreSnapshot,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WaitCounterSnapshot {
    pub available: bool,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub rank: i64,
    #[serde(default)]
    pub counters: Vec<WaitCounterRow>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WaitCounterRow {
    pub name: String,
    pub category: String,
    pub rank: i64,
    pub active_count: i64,
    pub total_calls: i64,
    pub total_time_us: i64,
    pub max_time_us: i64,
    pub avg_time_us: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TcpStoreSnapshot {
    pub available: bool,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub values_enabled: bool,
    #[serde(default)]
    pub catalog_available: bool,
    #[serde(default)]
    pub catalog_mode: String,
    #[serde(default)]
    pub total_keys: usize,
    #[serde(default)]
    pub identified_keys: usize,
    #[serde(default)]
    pub facts: Vec<TcpStoreFact>,
    #[serde(default)]
    pub entries: Vec<TcpStoreEntry>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TcpStoreFact {
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TcpStoreEntry {
    pub key: String,
    pub category: String,
    pub value_size: usize,
    pub value_preview: String,
    pub redacted: bool,
}

/// PyTorch Profiler API
impl ApiClient {
    pub async fn get_pytorch_runtime_debug(&self) -> Result<RuntimeDebugResponse> {
        let response = self
            .get_request("/apis/pythonext/pytorch/runtime-debug")
            .await?;
        Self::parse_json(&response)
    }

    /// Start PyTorch profiler, specify the number of steps to profile
    pub async fn start_pytorch_profile(&self, steps: i32) -> Result<ProfileResponse> {
        let path = format!("/apis/pythonext/pytorch/profile/start?steps={}", steps);
        let response = self.get_request(&path).await?;
        let result: ProfileResponse = Self::parse_json(&response)?;
        Ok(result)
    }

    /// Get PyTorch profiler timeline data (Chrome tracing format)
    pub async fn get_pytorch_timeline(&self) -> Result<String> {
        let path = "/apis/pythonext/pytorch/timeline";
        let response = self.get_request(path).await?;

        // Check if response is an error
        if let Ok(error_response) = serde_json::from_str::<serde_json::Value>(&response) {
            if let Some(error) = error_response.get("error") {
                let error_msg = error.as_str().unwrap_or("Unknown error").to_string();
                log::warn!("PyTorch timeline API returned error: {}", error_msg);
                return Err(crate::utils::error::AppError::Api(error_msg));
            }
        }

        // Validate if response is valid Chrome tracing format
        if let Ok(trace_data) = serde_json::from_str::<serde_json::Value>(&response) {
            if let Some(trace_events) = trace_data.get("traceEvents") {
                if trace_events
                    .as_array()
                    .map(|arr| arr.is_empty())
                    .unwrap_or(true)
                {
                    return Err(crate::utils::error::AppError::Api(
                        "Timeline data is empty. Make sure the profiler has been executed."
                            .to_string(),
                    ));
                }
            }
        }

        Ok(response)
    }
}
