use super::ApiClient;
use crate::utils::error::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
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
                return Err(crate::utils::error::AppError::Api(
                    error.as_str().unwrap_or("Unknown error").to_string()
                ));
            }
        }
        
        Ok(response)
    }
}

