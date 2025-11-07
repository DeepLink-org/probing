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

    /// 预览查询（带回退）：优先按第一列降序获取最近10条，失败则退化为 limit 10
    pub async fn execute_preview_last10(&self, table: &str) -> Result<DataFrame> {
        let try_sqls = [
            format!("select * from {} order by 1 desc limit 10", table),
            format!("select * from {} limit 10", table),
        ];
        let mut last_err: Option<AppError> = None;
        for sql in try_sqls {
            match self.execute_query(&sql).await {
                Ok(df) => return Ok(df),
                Err(e) => last_err = Some(e),
            }
        }
        Err(last_err.unwrap_or_else(|| AppError::Api("Preview query failed".to_string())))
    }
}