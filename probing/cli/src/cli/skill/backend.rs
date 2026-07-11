//! ``ProbeEndpoint`` adapter for the shared skill runner.

use probing_proto::prelude::{DataFrame, NodeListResponse, Query};
use probing_skills::backend::{parse_cluster_query_response, ClusterQueryMeta, SkillBackend};
use probing_skills::runner::{Result, SkillRunError};

use crate::cli::ctrl::ProbeEndpoint;

pub struct CliBackend(pub ProbeEndpoint);

#[async_trait::async_trait]
impl SkillBackend for CliBackend {
    async fn query_local(&self, sql: &str) -> Result<DataFrame> {
        self.0
            .query(Query::new(sql.to_string()))
            .await
            .map_err(|e| SkillRunError(e.to_string()))
    }

    async fn cluster_query(&self, sql: &str) -> Result<(DataFrame, Option<ClusterQueryMeta>)> {
        let body = serde_json::json!({
            "expr": sql,
            "cluster": true,
        });
        let reply = self
            .0
            .post_json("/apis/cluster/query", &body.to_string())
            .await
            .map_err(|e| SkillRunError(e.to_string()))?;
        let value: serde_json::Value =
            serde_json::from_str(&reply).map_err(|e| SkillRunError(e.to_string()))?;
        let (dataframe, cluster_meta) = parse_cluster_query_response(&value)?;
        Ok((dataframe, cluster_meta))
    }

    async fn get(&self, path: &str) -> Result<String> {
        self.0
            .get(path)
            .await
            .map_err(|e| SkillRunError(e.to_string()))
    }

    async fn peer_count(&self) -> usize {
        match self.0.get("/apis/nodes?limit=1024").await {
            Ok(reply) => match serde_json::from_str::<NodeListResponse>(&reply) {
                Ok(resp) => resp.total.saturating_sub(1),
                Err(_) => 0,
            },
            Err(_) => 0,
        }
    }
}
