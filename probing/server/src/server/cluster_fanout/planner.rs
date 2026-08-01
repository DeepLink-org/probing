use probing_core::core::cluster::{
    hierarchical_metadata_available, hierarchical_metadata_unavailable_err, local_leaf_peers,
    local_listen_addrs, node_aggregator_peers,
};
use probing_core::core::federation::{is_local0_from_env, FanoutScope};
use probing_proto::prelude::Node;

use super::types::ClusterFanoutScope;

#[derive(Debug, Clone, Copy)]
pub(super) struct FanoutPlan {
    pub hierarchical_requested: bool,
    pub scope: ClusterFanoutScope,
}

pub(super) fn plan_fanout(
    hierarchical: bool,
    requested_scope: ClusterFanoutScope,
) -> anyhow::Result<FanoutPlan> {
    let hierarchical_requested =
        hierarchical && probing_core::core::federation::hierarchical_fanout_enabled();
    let scope = resolve_scope(requested_scope, hierarchical_requested);

    if hierarchical_requested
        && is_local0_from_env()
        && matches!(
            scope,
            ClusterFanoutScope::Coordinator | ClusterFanoutScope::Node
        )
        && !hierarchical_metadata_available()
    {
        return Err(hierarchical_metadata_unavailable_err().into());
    }

    Ok(FanoutPlan {
        hierarchical_requested,
        scope,
    })
}

fn resolve_scope(requested: ClusterFanoutScope, hierarchical: bool) -> ClusterFanoutScope {
    match requested {
        ClusterFanoutScope::Auto if hierarchical && is_local0_from_env() => {
            ClusterFanoutScope::Coordinator
        }
        ClusterFanoutScope::Auto if hierarchical => ClusterFanoutScope::Local,
        ClusterFanoutScope::Auto if is_local0_from_env() => ClusterFanoutScope::Coordinator,
        ClusterFanoutScope::Auto => ClusterFanoutScope::Local,
        explicit => explicit,
    }
}

pub(super) fn peers_for_scope(scope: FanoutScope) -> Vec<Node> {
    match scope {
        FanoutScope::Coordinator => node_aggregator_peers(),
        FanoutScope::Node => local_leaf_peers(),
        FanoutScope::Flat | FanoutScope::Auto => {
            let local_addrs = local_listen_addrs();
            probing_core::core::cluster::get_nodes()
                .into_iter()
                .filter(probing_core::core::cluster::is_node_alive)
                .filter(|node| !local_addrs.contains(&node.addr))
                .collect()
        }
        FanoutScope::Local => Vec::new(),
    }
}
