//! Browser HTTP backend for the shared skill runner.

use probing_proto::prelude::{DataFrame, Message, NodeListResponse, Query, QueryDataFormat};
use probing_skills::backend::{ClusterQueryMeta, SkillBackend};
use probing_skills::runner::{Result, SkillRunError};

use crate::api::ApiClient;
use crate::utils::error::AppError;

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
        let meta = ClusterQueryMeta {
            partial: !resp.meta.nodes_failed.is_empty(),
            nodes_queried: resp.meta.nodes_queried,
            nodes_failed: resp.meta.nodes_failed.clone(),
            peer_batches_dropped: 0,
        };
        Ok((resp.dataframe, Some(meta)))
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

#[allow(dead_code)]
fn _query_envelope_unused() {
    let _ = Message::new(Query::default());
    let _ = QueryDataFormat::Nil;
    let _ = NodeListResponse::default();
    let _ = AppError::Api(String::new());
}
