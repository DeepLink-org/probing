use super::ApiClient;
use crate::utils::error::{AppError, Result};
use probing_proto::prelude::*;

/// 性能分析API
impl ApiClient {
    /// 获取profiler配置
    pub async fn get_profiler_config(&self) -> Result<Vec<Vec<String>>> {
        let request = Message::new(Query {
            expr: "select name, value from information_schema.df_settings where name like 'probing.%';".to_string(),
            ..Default::default()
        });
        
        let response = self.post_request("/query", &request).await?;
        
        // Parse DataFrame structure
        let json: serde_json::Value = Self::parse_json(&response)?;
        let data: Vec<Vec<String>> = json["payload"]["DataFrame"]["data"]
            .as_array()
            .ok_or_else(|| AppError::Api("Invalid DataFrame structure".to_string()))?
            .iter()
            .map(|row| {
                row.as_array()
                    .unwrap()
                    .iter()
                    .map(|cell| cell.as_str().unwrap_or("").to_string())
                    .collect()
            })
            .collect();
        
        Ok(data)
    }

    /// 获取火焰图数据
    pub async fn get_flamegraph(&self, profiler_type: &str) -> Result<String> {
        self.get_request(&format!("/apis/flamegraph/{}", profiler_type)).await
    }
}
