//! Cluster / distributed diagnostic context for the Agent.

use probing_proto::prelude::Node;

use crate::api::{ApiClient, ClusterQueryMeta};
use crate::utils::error::Result;

#[derive(Debug, Clone, Default)]
pub struct ClusterSnapshot {
    pub node_count: usize,
    /// Peers excluding the local coordinator (same notion as Training page scan).
    pub peer_count: usize,
    pub nodes_summary: String,
}

impl ClusterSnapshot {
    pub fn has_peers(&self) -> bool {
        self.peer_count > 0
    }

    pub fn is_distributed(&self) -> bool {
        self.node_count > 1 || self.peer_count > 0
    }
}

fn node_is_healthy(node: &Node) -> bool {
    matches!(
        node.status.as_deref().unwrap_or("").to_lowercase().as_str(),
        "ok" | "healthy" | "running" | "ready" | "online"
    )
}

pub async fn fetch_cluster_snapshot() -> ClusterSnapshot {
    match ApiClient::new().get_nodes().await {
        Ok(nodes) => snapshot_from_nodes(&nodes),
        Err(e) => ClusterSnapshot {
            nodes_summary: format!("(cluster.nodes unavailable: {})", e.display_message()),
            ..Default::default()
        },
    }
}

fn snapshot_from_nodes(nodes: &[Node]) -> ClusterSnapshot {
    let node_count = nodes.len();
    let peer_count = node_count.saturating_sub(1);
    let healthy_count = nodes.iter().filter(|n| node_is_healthy(n)).count();
    let world_size = nodes.first().and_then(|n| n.world_size);

    let mut lines: Vec<String> = Vec::new();
    if node_count == 0 {
        lines.push("No cluster nodes registered (standalone mode).".to_string());
    } else {
        lines.push(format!(
            "Cluster view: {node_count} node(s), {peer_count} peer(s), {healthy_count} healthy"
        ));
        if let Some(ws) = world_size {
            lines.push(format!("World size: {ws}"));
        }
        for node in nodes.iter().take(12) {
            let rank = node
                .rank
                .map(|r| r.to_string())
                .unwrap_or_else(|| "—".to_string());
            let status = node.status.clone().unwrap_or_else(|| "?".to_string());
            lines.push(format!(
                "- rank {rank} {} {} [{status}]",
                node.host, node.addr
            ));
        }
        if node_count > 12 {
            lines.push(format!("… +{} more nodes", node_count - 12));
        }
    }

    ClusterSnapshot {
        node_count,
        peer_count,
        nodes_summary: lines.join("\n"),
    }
}

/// Whether this SQL should fan out via `/apis/cluster/query`.
pub fn sql_needs_cluster_fanout(sql: &str, step_cluster: bool) -> bool {
    step_cluster || sql.to_lowercase().contains("global.")
}

pub fn format_cluster_meta(meta: &ClusterQueryMeta) -> String {
    if !meta.cluster {
        return "local query".to_string();
    }
    let mut note = format!("cluster fan-out · {} nodes queried", meta.nodes_queried);
    if !meta.nodes_failed.is_empty() {
        note.push_str(&format!(
            " · {} node(s) failed: {}",
            meta.nodes_failed.len(),
            meta.nodes_failed.join(", ")
        ));
    }
    note
}

pub fn cluster_context_for_llm(snapshot: &ClusterSnapshot) -> String {
    let mode = if snapshot.is_distributed() {
        "distributed (use global.* / cluster fan-out for cross-node SQL)"
    } else {
        "standalone (local probe tables only; set use_global=false)"
    };
    format!("Cluster mode: {mode}\n{}", snapshot.nodes_summary)
}

/// Default `use_global` when the playbook parameter is not overridden.
pub fn default_use_global(snapshot: &ClusterSnapshot, playbook_default: bool) -> bool {
    if !snapshot.is_distributed() {
        return false;
    }
    playbook_default
}

pub async fn execute_sql_for_agent(
    sql: &str,
    cluster_fanout: bool,
) -> Result<(probing_proto::prelude::DataFrame, Option<ClusterQueryMeta>)> {
    let client = ApiClient::new();
    if cluster_fanout {
        let resp = client.cluster_query(sql, true).await?;
        Ok((resp.dataframe, Some(resp.meta)))
    } else {
        let df = client.execute_query(sql).await?;
        Ok((df, None))
    }
}
