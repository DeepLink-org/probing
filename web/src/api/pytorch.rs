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

/// PyTorch Profiler API
impl ApiClient {
    /// 启动 PyTorch profiler，指定要 profile 的 step 数量
    pub async fn start_pytorch_profile(&self, steps: i32) -> Result<ProfileResponse> {
        let path = format!("/apis/pythonext/pytorch/profile?steps={}", steps);
        let response = self.get_request(&path).await?;
        let result: ProfileResponse = Self::parse_json(&response)?;
        Ok(result)
    }

    /// 获取 PyTorch profiler 的 timeline 数据（Chrome tracing 格式）
    pub async fn get_pytorch_timeline(&self) -> Result<String> {
        let path = "/apis/pythonext/pytorch/timeline";
        let response = self.get_request(path).await?;
        
        // 检查是否是错误响应
        if let Ok(error_response) = serde_json::from_str::<serde_json::Value>(&response) {
            if let Some(error) = error_response.get("error") {
                let error_msg = error.as_str().unwrap_or("Unknown error").to_string();
                log::warn!("PyTorch timeline API returned error: {}", error_msg);
                return Err(crate::utils::error::AppError::Api(error_msg));
            }
        }
        
        // 验证响应是否是有效的 Chrome tracing 格式
        if let Ok(trace_data) = serde_json::from_str::<serde_json::Value>(&response) {
            if let Some(trace_events) = trace_data.get("traceEvents") {
                if trace_events.as_array().map(|arr| arr.is_empty()).unwrap_or(true) {
                    return Err(crate::utils::error::AppError::Api(
                        "Timeline data is empty. Make sure the profiler has been executed.".to_string()
                    ));
                }
            }
        }
        
        Ok(response)
    }
}

