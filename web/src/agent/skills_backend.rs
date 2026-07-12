//! Browser HTTP backend for the shared skill runner.

use probing_proto::prelude::DataFrame;
use probing_skills::backend::{parse_cluster_query_response, ClusterQueryMeta, SkillBackend};
use probing_skills::runner::{Result, SkillRunError};

use crate::api::ApiClient;

pub struct WebBackend;

#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
impl SkillBackend for WebBackend {
    async fn query_local(&self, sql: &str) -> Result<DataFrame> {
        let client = ApiClient::new();
        client
            .execute_query(sql)
            .await
            .map_err(|e| SkillRunError(e.display_message()))
    }

    async fn cluster_query(&self, sql: &str) -> Result<(DataFrame, Option<ClusterQueryMeta>)> {
        let client = ApiClient::new();
        let resp = client
            .cluster_query(sql, true)
            .await
            .map_err(|e| SkillRunError(e.display_message()))?;
        let value = serde_json::json!({
            "dataframe": resp.dataframe,
            "meta": resp.meta,
        });
        let (dataframe, meta) = parse_cluster_query_response(&value)?;
        Ok((dataframe, meta))
    }

    async fn get(&self, path: &str) -> Result<String> {
        let client = ApiClient::new();
        if path.contains("callstack") {
            let frames = client
                .get_callstack_with_mode(None, "mixed")
                .await
                .map_err(|e| SkillRunError(e.display_message()))?;
            return Ok(frames
                .iter()
                .take(24)
                .map(|f| format!("{f}"))
                .collect::<Vec<_>>()
                .join("\n"));
        }
        if path.contains("/apis/nodes") || path == "/apis/nodes" {
            let nodes = client
                .get_nodes()
                .await
                .map_err(|e| SkillRunError(e.display_message()))?;
            return Ok(nodes
                .iter()
                .map(|n| {
                    format!(
                        "rank={} host={} addr={} status={}",
                        n.rank
                            .map(|r| r.to_string())
                            .unwrap_or_else(|| "—".to_string()),
                        n.host,
                        n.addr,
                        n.status.clone().unwrap_or_else(|| "?".to_string())
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"));
        }
        client
            .get_raw(path)
            .await
            .map_err(|e| SkillRunError(e.display_message()))
    }

    async fn peer_count(&self) -> usize {
        match ApiClient::new().get_nodes().await {
            Ok(nodes) => nodes.len().saturating_sub(1),
            Err(_) => 0,
        }
    }
}
