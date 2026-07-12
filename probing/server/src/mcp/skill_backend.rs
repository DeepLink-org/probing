//! In-process skill backend for MCP (no HTTP/CLI loopback).

use async_trait::async_trait;
use probing_core::core::cluster::get_nodes_page;
use probing_proto::prelude::{DataFrame, NodeListResponse, Query, QueryDataFormat};
use probing_skills::backend::{ClusterQueryMeta, SkillBackend};
use probing_skills::runner::{Result, SkillRunError};

use crate::engine::handle_query;
use crate::server::cluster_fanout::{self, ClusterFanoutScope};

use super::helpers;

pub struct ServerBackend;

#[async_trait]
impl SkillBackend for ServerBackend {
    async fn query_local(&self, sql: &str) -> Result<DataFrame> {
        let reply = handle_query(Query {
            expr: sql.to_string(),
            opts: None,
        })
        .await
        .map_err(|e| SkillRunError(e.to_string()))?;
        match reply {
            QueryDataFormat::DataFrame(df) => Ok(df),
            QueryDataFormat::Nil => Ok(DataFrame::default()),
            QueryDataFormat::Error(err) => {
                Err(SkillRunError(format!("{:?}: {}", err.code, err.message)))
            }
            QueryDataFormat::TimeSeries(_) => Err(SkillRunError(
                "TimeSeries responses are not supported".into(),
            )),
        }
    }

    async fn cluster_query(&self, sql: &str) -> Result<(DataFrame, Option<ClusterQueryMeta>)> {
        let result = cluster_fanout::fanout_query(sql, true, true, ClusterFanoutScope::Auto)
            .await
            .map_err(|e| SkillRunError(format!("{e:#}")))?;
        let meta = ClusterQueryMeta {
            partial: result.meta.partial,
            nodes_queried: result.meta.nodes_queried,
            nodes_failed: result.meta.nodes_failed.clone(),
            peer_batches_dropped: result.meta.peer_batches_dropped,
        };
        Ok((result.dataframe, Some(meta)))
    }

    async fn get(&self, path: &str) -> Result<String> {
        if path.contains("/apis/nodes") || path == "/apis/nodes" {
            let (version, total, nodes) =
                tokio::task::spawn_blocking(|| get_nodes_page(0, 10_000, None))
                    .await
                    .map_err(|e| SkillRunError(e.to_string()))?;
            let resp = NodeListResponse {
                version,
                total,
                offset: 0,
                nodes,
            };
            return serde_json::to_string(&resp).map_err(|e| SkillRunError(e.to_string()));
        }
        let bytes = helpers::extension_request(path, b"")
            .await
            .map_err(|e| SkillRunError(e.message.to_string()))?;
        String::from_utf8(bytes).map_err(|e| SkillRunError(e.to_string()))
    }

    async fn peer_count(&self) -> usize {
        match tokio::task::spawn_blocking(|| get_nodes_page(0, 1024, None)).await {
            Ok((_, total, _)) => total.saturating_sub(1),
            Err(_) => 0,
        }
    }
}
