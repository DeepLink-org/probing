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

/// Parse `POST /apis/cluster/query` JSON envelope into dataframe + optional meta.
pub fn parse_cluster_query_response(
    value: &serde_json::Value,
) -> std::result::Result<(DataFrame, Option<ClusterQueryMeta>), SkillRunError> {
    if let Some(err) = value.get("error").and_then(|v| v.as_str()) {
        return Err(SkillRunError(err.to_string()));
    }
    let df = value
        .get("dataframe")
        .ok_or_else(|| SkillRunError("missing dataframe in cluster response".to_string()))?;
    let dataframe: DataFrame =
        serde_json::from_value(df.clone()).map_err(|e| SkillRunError(e.to_string()))?;
    let cluster_meta = value.get("meta").map(parse_cluster_meta);
    Ok((dataframe, cluster_meta))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_cluster_query_response_surfaces_api_error() {
        let value = serde_json::json!({"error": "boom"});
        let err = parse_cluster_query_response(&value).unwrap_err();
        assert_eq!(err.0, "boom");
    }

    #[test]
    fn parse_cluster_query_response_extracts_dataframe_and_meta() {
        let dataframe = DataFrame::default();
        let value = serde_json::json!({
            "dataframe": dataframe,
            "meta": {
                "nodes_queried": 2,
                "nodes_failed": [],
                "peer_batches_dropped": 0,
                "partial": false,
            },
        });
        let (df, meta) = parse_cluster_query_response(&value).unwrap();
        assert_eq!(df.row_count(), 0);
        assert_eq!(meta.unwrap().nodes_queried, 2);
    }

    #[test]
    fn parse_cluster_meta_marks_partial_on_failed_nodes() {
        let meta = parse_cluster_meta(&serde_json::json!({
            "nodes_queried": 8,
            "nodes_failed": ["rank-2"],
            "peer_batches_dropped": 0,
            "partial": false,
        }));
        assert!(meta.partial);
        assert_eq!(meta.nodes_failed, vec!["rank-2".to_string()]);
    }

    #[test]
    fn parse_cluster_meta_honours_explicit_partial_flag() {
        let meta = parse_cluster_meta(&serde_json::json!({
            "nodes_queried": 4,
            "nodes_failed": [],
            "peer_batches_dropped": 2,
            "partial": true,
        }));
        assert!(meta.partial);
        assert_eq!(meta.peer_batches_dropped, 2);
    }

    #[test]
    fn cluster_meta_note_includes_partial_marker() {
        let note = cluster_meta_note(&ClusterQueryMeta {
            partial: true,
            nodes_queried: 2,
            nodes_failed: vec!["a".into()],
            peer_batches_dropped: 1,
        });
        assert!(note.contains("PARTIAL"));
        assert!(note.contains("2 nodes"));
    }
}
