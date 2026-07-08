//! HTTP/query backend for skill step execution.

use async_trait::async_trait;
use probing_proto::prelude::DataFrame;

use crate::runner::SkillRunError;

pub type Result<T> = std::result::Result<T, SkillRunError>;

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ClusterQueryMeta {
    pub partial: bool,
    pub nodes_queried: usize,
    pub nodes_failed: Vec<String>,
    pub peer_batches_dropped: usize,
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
pub trait SkillBackend {
    async fn query_local(&self, sql: &str) -> Result<DataFrame>;

    async fn cluster_query(&self, sql: &str) -> Result<(DataFrame, Option<ClusterQueryMeta>)>;

    async fn get(&self, path: &str) -> Result<String>;

    async fn peer_count(&self) -> usize;
}

pub fn parse_cluster_meta(meta: &serde_json::Value) -> ClusterQueryMeta {
    let nodes_failed: Vec<String> = meta
        .get("nodes_failed")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let peer_batches_dropped = meta
        .get("peer_batches_dropped")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;
    let nodes_queried = meta
        .get("nodes_queried")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;
    let partial = meta
        .get("partial")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || !nodes_failed.is_empty()
        || peer_batches_dropped > 0;
    ClusterQueryMeta {
        partial,
        nodes_queried,
        nodes_failed,
        peer_batches_dropped,
    }
}

pub fn cluster_meta_note(meta: &ClusterQueryMeta) -> String {
    format!(
        "cluster fan-out · {} nodes queried · {} failed · {} peer batches dropped{}",
        meta.nodes_queried,
        meta.nodes_failed.len(),
        meta.peer_batches_dropped,
        if meta.partial { " · PARTIAL" } else { "" }
    )
}
