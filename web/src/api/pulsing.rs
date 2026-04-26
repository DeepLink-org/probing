use super::ApiClient;
use crate::utils::error::Result;
use probing_proto::prelude::*;

impl ApiClient {
    pub async fn fetch_pulsing_actors(&self) -> Result<DataFrame> {
        self.execute_query("SELECT * FROM pulsing.actors").await
    }

    pub async fn fetch_pulsing_spans(&self) -> Result<DataFrame> {
        self.execute_query(
            "SELECT trace_id, span_id, parent_span_id, \
             name, kind, start_us, end_us, duration_us, status_code, \
             attr_actor_name, attr_pulsing_op \
             FROM pulsing.spans ORDER BY start_us DESC LIMIT 500",
        )
        .await
    }

    pub async fn fetch_pulsing_span_count(&self) -> Result<DataFrame> {
        self.execute_query("SELECT COUNT(*) AS total FROM pulsing.spans")
            .await
    }

    pub async fn fetch_pulsing_metrics(&self) -> Result<DataFrame> {
        self.execute_query(
            "SELECT * FROM pulsing.metrics ORDER BY timestamp_us DESC LIMIT 100",
        )
        .await
    }

    pub async fn fetch_pulsing_members(&self) -> Result<DataFrame> {
        self.execute_query("SELECT * FROM pulsing.members").await
    }
}
