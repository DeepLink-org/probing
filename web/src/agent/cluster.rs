//! Cluster / distributed diagnostic context for the Agent.

use probing_proto::prelude::Node;

use crate::api::ApiClient;

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

pub fn cluster_context_for_llm(snapshot: &ClusterSnapshot) -> String {
    let mode = if snapshot.is_distributed() {
        "distributed (use global.* / cluster fan-out for cross-node SQL)"
    } else {
        "standalone (local probe tables only; set use_global=false)"
    };
    format!("Cluster mode: {mode}\n{}", snapshot.nodes_summary)
}
