use super::ApiClient;
use crate::utils::error::{AppError, Result};
use probing_proto::prelude::*;

/// Time series analysis API
impl ApiClient {
    /// Execute SQL query
    pub async fn execute_query(&self, query: &str) -> Result<DataFrame> {
        self.execute_query_at_path("/query", query).await
    }

    /// Execute SQL query against another local probing process via the current server.
    pub async fn execute_query_local_pid(&self, pid: i32, query: &str) -> Result<DataFrame> {
        self.execute_query_at_path(&format!("/apis/query/local-pid?pid={pid}"), query)
            .await
    }

    async fn execute_query_at_path(&self, path: &str, query: &str) -> Result<DataFrame> {
        let request = Message::new(Query {
            expr: query.to_string(),
            ..Default::default()
        });

        let request_body = serde_json::to_string(&request)
            .map_err(|e| AppError::Api(format!("Failed to serialize request: {}", e)))?;

        let response = self.post_request_with_body(path, request_body).await?;

        let msg: Message<QueryDataFormat> = Self::parse_json(&response)?;

        match msg.payload {
            QueryDataFormat::DataFrame(dataframe) => Ok(dataframe),
            QueryDataFormat::Nil => Ok(DataFrame {
                names: vec![],
                cols: vec![],
                size: 0,
            }),
            QueryDataFormat::Error(err) => Err(AppError::Api(err.message)),
            QueryDataFormat::TimeSeries(_) => {
                Err(AppError::Api("TimeSeries format not supported".to_string()))
            }
        }
    }
}
