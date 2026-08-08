use super::ApiClient;
use crate::utils::error::{AppError, Result};
use probing_proto::prelude::*;

/// Time series analysis API
impl ApiClient {
    /// Execute SQL query
    pub async fn execute_query(&self, query: &str) -> Result<DataFrame> {
        let outcome = self.execute_query_outcome(query).await?;
        if outcome.quality.is_partial() {
            return Err(AppError::Api(format!(
                "query returned partial data: {} peer(s) failed, {} batch(es) dropped",
                outcome.quality.nodes_failed.len(),
                outcome.quality.peer_batches_dropped
            )));
        }
        Ok(outcome.data)
    }

    /// Execute SQL while preserving distributed completeness as typed data.
    pub async fn execute_query_outcome(&self, query: &str) -> Result<QueryOutcome<DataFrame>> {
        self.execute_query_outcome_at_path("/query", query).await
    }

    /// Execute SQL query against another local probing process via the current server.
    pub async fn execute_query_local_pid(&self, pid: i32, query: &str) -> Result<DataFrame> {
        let outcome = self
            .execute_query_outcome_at_path(&format!("/apis/query/local-pid?pid={pid}"), query)
            .await?;
        if outcome.quality.is_partial() {
            return Err(AppError::Api(
                "local query returned partial data".to_string(),
            ));
        }
        Ok(outcome.data)
    }

    async fn execute_query_outcome_at_path(
        &self,
        path: &str,
        query: &str,
    ) -> Result<QueryOutcome<DataFrame>> {
        let request = Message::new(Query {
            expr: query.to_string(),
            ..Default::default()
        });

        let request_body = serde_json::to_string(&request)
            .map_err(|e| AppError::Api(format!("Failed to serialize request: {}", e)))?;

        let response = self
            .post_request_with_body_accepting(path, request_body, &[503])
            .await?;

        let msg: Message<QueryDataFormat> = Self::parse_json(&response)?;
        let quality = msg.meta.and_then(|meta| meta.fanout).unwrap_or_default();

        let data = match msg.payload {
            QueryDataFormat::DataFrame(dataframe) => dataframe,
            QueryDataFormat::Nil => DataFrame {
                names: vec![],
                cols: vec![],
                size: 0,
            },
            QueryDataFormat::Error(err) => return Err(AppError::Api(err.message)),
            QueryDataFormat::TimeSeries(_) => {
                return Err(AppError::Api("TimeSeries format not supported".to_string()));
            }
        };
        Ok(QueryOutcome::with_quality(data, quality))
    }
}
