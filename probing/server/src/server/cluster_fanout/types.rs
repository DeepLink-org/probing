use probing_core::core::federation::{enforce_fanout_strict, FanoutStats};
use probing_proto::prelude::DataFrame;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClusterFanoutScope {
    #[default]
    Auto,
    Coordinator,
    Node,
    Local,
}

impl ClusterFanoutScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Coordinator => "coordinator",
            Self::Node => "node",
            Self::Local => "local",
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FanoutMeta {
    pub cluster: bool,
    pub hierarchical: bool,
    pub scope: String,
    /// Number of rank/endpoints attempted across the complete fan-out tree.
    pub nodes_queried: usize,
    pub nodes_failed: Vec<String>,
    /// Partial peer batches dropped while merging aggregate pushdown results.
    #[serde(default)]
    pub peer_batches_dropped: usize,
    pub node_aggregators_queried: usize,
    pub local_ranks_queried: usize,
    /// True when some peers failed or merge dropped partial batches — dataframe is incomplete.
    #[serde(default)]
    pub partial: bool,
}

impl FanoutMeta {
    pub(super) fn empty(cluster: bool, hierarchical: bool, scope: ClusterFanoutScope) -> Self {
        Self {
            cluster,
            hierarchical,
            scope: scope.as_str().into(),
            nodes_queried: 0,
            nodes_failed: Vec::new(),
            peer_batches_dropped: 0,
            node_aggregators_queried: 0,
            local_ranks_queried: 0,
            partial: false,
        }
    }

    pub(super) fn local(cluster: bool, hierarchical: bool, scope: ClusterFanoutScope) -> Self {
        let mut meta = Self::empty(cluster, hierarchical, scope);
        meta.nodes_queried = 1;
        meta
    }

    pub(super) fn record_peer_success(&mut self) {
        self.nodes_queried += 1;
    }

    pub(super) fn absorb(&mut self, child: FanoutMeta) {
        self.nodes_queried += child.nodes_queried;
        self.nodes_failed.extend(child.nodes_failed);
        self.peer_batches_dropped += child.peer_batches_dropped;
        self.partial |= child.partial;
    }

    pub(super) fn record_peer_failure(&mut self, addr: &str, error: &anyhow::Error) {
        self.nodes_queried += 1;
        self.nodes_failed.push(format!("{addr}: {error:#}"));
    }

    pub(super) fn finalize(&mut self) {
        self.partial =
            self.partial || !self.nodes_failed.is_empty() || self.peer_batches_dropped > 0;
    }

    fn fanout_stats(&self) -> FanoutStats {
        FanoutStats {
            nodes_succeeded: self.nodes_queried.saturating_sub(self.nodes_failed.len()),
            nodes_failed: self.nodes_failed.clone(),
            peer_batches_dropped: self.peer_batches_dropped,
            partial: self.partial,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FanoutOutcome {
    pub dataframe: DataFrame,
    pub meta: FanoutMeta,
}

// Compatibility name retained for the existing HTTP/MCP and regression contracts.
pub use FanoutOutcome as FanoutQueryResponse;

pub(super) fn finish_fanout(
    dataframe: DataFrame,
    mut meta: FanoutMeta,
    context: &str,
) -> anyhow::Result<FanoutOutcome> {
    meta.finalize();
    if meta.partial {
        log::warn!(
            "cluster fan-out partial ({context}): nodes_queried={} nodes_failed={} peer_batches_dropped={}",
            meta.nodes_queried,
            meta.nodes_failed.len(),
            meta.peer_batches_dropped,
        );
        enforce_fanout_strict(&meta.fanout_stats()).map_err(|error| anyhow::anyhow!("{error}"))?;
    }
    Ok(FanoutOutcome { dataframe, meta })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fanout_meta_partial_when_peers_fail() {
        let mut meta = FanoutMeta {
            cluster: true,
            hierarchical: true,
            scope: "flat".into(),
            nodes_queried: 10,
            nodes_failed: vec!["10.0.0.2:8080".into()],
            peer_batches_dropped: 0,
            node_aggregators_queried: 0,
            local_ranks_queried: 0,
            partial: false,
        };
        meta.finalize();
        assert!(meta.partial);

        let mut clean = meta.clone();
        clean.nodes_failed.clear();
        clean.partial = false;
        clean.finalize();
        assert!(!clean.partial);

        clean.peer_batches_dropped = 2;
        clean.finalize();
        assert!(clean.partial);
    }
}
