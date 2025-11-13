use crate::utils::error::{AppError, Result};

/// 基础API客户端
pub struct ApiClient;

impl ApiClient {
    pub fn new() -> Self {
        Self
    }

    /// 获取当前页面的origin
    fn get_origin() -> Result<String> {
        web_sys::window()
            .ok_or_else(|| AppError::Api("No window object".to_string()))?
            .location()
            .origin()
            .map_err(|_| AppError::Api("Failed to get origin".to_string()))
    }

    /// 构建API URL
    fn build_url(path: &str) -> Result<String> {
        Ok(format!("{}{}", Self::get_origin()?, path))
    }

    /// 发送GET请求
    async fn get_request(&self, path: &str) -> Result<String> {
        let url = Self::build_url(path)?;
        let response = reqwest::get(&url).await?;
        
        if !response.status().is_success() {
            return Err(AppError::Api(format!("HTTP error: {}", response.status())));
        }

        response.text().await.map_err(|e| AppError::Api(e.to_string()))
    }

    /// 发送POST请求（自定义Content-Type）
    async fn post_request_with_body(&self, path: &str, body: String) -> Result<String> {
        let url = Self::build_url(path)?;
        let client = reqwest::Client::new();
        let response = client
            .post(&url)
            .body(body)
            .header("Content-Type", "application/json")
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(AppError::Api(format!("HTTP error: {}", response.status())));
        }

        response.text().await.map_err(|e| AppError::Api(e.to_string()))
    }

    /// 解析JSON响应
    fn parse_json<T: serde::de::DeserializeOwned>(response: &str) -> Result<T> {
        serde_json::from_str(response)
            .map_err(|e| AppError::Api(format!("JSON parse error: {}", e)))
    }
}

// 导出所有API模块
mod analytics;
mod cluster;
mod dashboard;
mod profiling;
mod pytorch;
mod stack;
mod trace;
mod traces;

#[allow(unused_imports)]
pub use analytics::*;
#[allow(unused_imports)]
pub use cluster::*;
#[allow(unused_imports)]
pub use dashboard::*;
#[allow(unused_imports)]
pub use profiling::*;
#[allow(unused_imports)]
pub use pytorch::*;
#[allow(unused_imports)]
pub use stack::*;
#[allow(unused_imports)]
pub use trace::*;
#[allow(unused_imports)]
pub use traces::*;