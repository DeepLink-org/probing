use super::ApiClient;
use crate::utils::error::Result;
use serde::{Deserialize, Serialize};

/// Trace API 响应结构
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceResponse {
    pub success: bool,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

/// Trace 状态信息（简化版，因为 show_trace 只返回函数名列表）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceInfo {
    pub function: String,
}

/// Trace API
impl ApiClient {
    /// 获取可追踪的函数列表（返回函数名列表）
    pub async fn get_traceable_functions(&self, prefix: Option<&str>) -> Result<Vec<String>> {
        let base = "/apis/pythonext/trace/list";
        let path = if let Some(prefix) = prefix {
            format!("{}?prefix={}", base, prefix)
        } else {
            base.to_string()
        };
        let response = self.get_request(&path).await?;
        let functions: Vec<String> = Self::parse_json(&response)?;
        Ok(functions)
    }

    /// 获取当前 trace 状态（返回已追踪的函数名列表）
    pub async fn get_trace_info(&self) -> Result<Vec<String>> {
        let path = "/apis/pythonext/trace/show";
        let response = self.get_request(path).await?;
        let info: Vec<String> = Self::parse_json(&response)?;
        Ok(info)
    }

    /// 开始追踪函数
    pub async fn start_trace(
        &self,
        function: &str,
        watch: Option<Vec<String>>,
        depth: Option<i32>,
    ) -> Result<TraceResponse> {
        let base = "/apis/pythonext/trace/start";
        let mut params = vec![format!("function={}", function)];
        
        if let Some(watch) = watch {
            if !watch.is_empty() {
                params.push(format!("watch={}", watch.join(",")));
            }
        }
        
        if let Some(depth) = depth {
            params.push(format!("depth={}", depth));
        }
        
        let path = if params.len() > 1 {
            format!("{}?{}", base, params.join("&"))
        } else {
            format!("{}?function={}", base, function)
        };
        
        let response = self.get_request(&path).await?;
        let result: TraceResponse = Self::parse_json(&response)?;
        Ok(result)
    }

    /// 停止追踪函数
    pub async fn stop_trace(&self, function: &str) -> Result<TraceResponse> {
        let path = format!("/apis/pythonext/trace/stop?function={}", function);
        let response = self.get_request(&path).await?;
        let result: TraceResponse = Self::parse_json(&response)?;
        Ok(result)
    }
}

