use super::ApiClient;
use crate::utils::error::{AppError, Result};
use probing_proto::prelude::*;

/// 时间序列分析API
impl ApiClient {
    /// 执行SQL查询
    pub async fn execute_query(&self, query: &str) -> Result<DataFrame> {
        let request = Message::new(Query {
            expr: query.to_string(),
            ..Default::default()
        });
        
        let request_body = serde_json::to_string(&request)
            .map_err(|e| AppError::Api(format!("Failed to serialize request: {}", e)))?;
        
        let response = self.post_request_with_body("/query", request_body).await?;
        
        // Parse Message<QueryDataFormat>
        let msg: Message<QueryDataFormat> = Self::parse_json(&response)?;
        
        match msg.payload {
            QueryDataFormat::DataFrame(dataframe) => Ok(dataframe),
            _ => Err(AppError::Api("Bad Response: DataFrame is Expected.".to_string()))
        }
    }
}