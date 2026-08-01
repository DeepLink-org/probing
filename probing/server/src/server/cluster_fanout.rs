//! On-demand SQL fan-out orchestration across cluster nodes.
//!
//! Planning, peer transport, bounded execution, and dataframe merging live in focused
//! submodules. This facade retains the existing HTTP/MCP-facing contract.

mod executor;
mod merge;
mod planner;
mod transport;
mod types;

use probing_core::core::cluster::{local_leaf_peers, node_aggregator_peers};
use probing_core::core::federation::{
    can_fanout_via_global_catalog, cluster_rank_for_endpoint, is_local0_from_env,
    reset_fanout_stats, rewrite_sql_for_global_fanout, take_fanout_stats, validate_global_query,
    with_fanout_scope_async, FanoutScope,
};
use probing_proto::prelude::*;

use crate::engine::handle_query;
use executor::{query_leaf_peers, query_node_peers};
use merge::{merge_tagged_dataframes, tag_dataframe};
use planner::{peers_for_scope, plan_fanout};
use transport::HttpPeerQueryClient;
use types::{finish_fanout, FanoutOutcome};

pub use transport::remote_query_df;
pub use types::{ClusterFanoutScope, FanoutMeta, FanoutQueryResponse};

fn local_host_label() -> String {
    crate::report::get_hostname().unwrap_or_else(|_| "localhost".into())
}

pub async fn query_local_df(sql: &str) -> anyhow::Result<DataFrame> {
    match handle_query(Query {
        expr: sql.to_string(),
        ..Default::default()
    })
    .await?
    {
        QueryDataFormat::DataFrame(dataframe) => Ok(dataframe),
        QueryDataFormat::Nil => Ok(DataFrame::default()),
        QueryDataFormat::Error(error) => anyhow::bail!("query error: {}", error.message),
        QueryDataFormat::TimeSeries(_) => anyhow::bail!("unexpected timeseries"),
    }
}

async fn query_local_df_in_scope(scope: FanoutScope, sql: &str) -> anyhow::Result<DataFrame> {
    with_fanout_scope_async(scope, query_local_df(sql)).await
}

/// Run `sql` locally, optionally fanning out to peer nodes in the cluster view.
pub async fn fanout_query(
    sql: &str,
    cluster: bool,
    hierarchical: bool,
    scope: ClusterFanoutScope,
) -> anyhow::Result<FanoutQueryResponse> {
    with_fanout_scope_async(
        FanoutScope::Auto,
        fanout_query_in_context(sql, cluster, hierarchical, scope),
    )
    .await
}

async fn fanout_query_in_context(
    sql: &str,
    cluster: bool,
    hierarchical: bool,
    scope: ClusterFanoutScope,
) -> anyhow::Result<FanoutOutcome> {
    if cluster {
        validate_global_query(sql)?;
    } else {
        return Ok(FanoutOutcome {
            dataframe: query_local_df(sql).await?,
            meta: FanoutMeta::local(false, hierarchical, ClusterFanoutScope::Local),
        });
    }

    let plan = plan_fanout(hierarchical, scope)?;
    match plan.scope {
        ClusterFanoutScope::Local => finish_fanout(
            query_local_df(sql).await?,
            FanoutMeta::local(true, hierarchical, ClusterFanoutScope::Local),
            "local",
        ),
        ClusterFanoutScope::Node => fanout_node_tier(sql, hierarchical).await,
        ClusterFanoutScope::Coordinator => {
            fanout_coordinator_tier(sql, plan.hierarchical_requested).await
        }
        ClusterFanoutScope::Auto => unreachable!("planner must resolve auto fan-out scope"),
    }
}

/// Node aggregator: local0 + on-node leaf ranks.
async fn fanout_node_tier(sql: &str, hierarchical: bool) -> anyhow::Result<FanoutOutcome> {
    if !is_local0_from_env() {
        return finish_fanout(
            query_local_df(sql).await?,
            FanoutMeta::local(true, hierarchical, ClusterFanoutScope::Local),
            "node-tier-local-fallback",
        );
    }

    let host = local_host_label();
    let addr = probing_core::core::cluster::local_addr_label();
    let rank = cluster_rank_for_endpoint(&host, &addr);
    let mut parts = vec![tag_dataframe(
        query_local_df_in_scope(FanoutScope::Node, sql).await?,
        &host,
        &addr,
        rank,
    )];

    let leaves = local_leaf_peers();
    let mut meta = FanoutMeta::local(true, hierarchical, ClusterFanoutScope::Node);
    meta.local_ranks_queried = leaves.len();
    let client = HttpPeerQueryClient;
    for (node, result) in query_leaf_peers(&client, leaves, sql).await {
        match result {
            Ok(dataframe) => {
                meta.record_peer_success();
                parts.push(tag_dataframe(
                    dataframe,
                    if node.host.is_empty() {
                        &node.addr
                    } else {
                        &node.host
                    },
                    &node.addr,
                    node.rank,
                ));
            }
            Err(error) => {
                log::warn!("local leaf fan-out {} failed: {error:#}", node.addr);
                meta.record_peer_failure(&node.addr, &error);
            }
        }
    }

    finish_fanout(merge_tagged_dataframes(&parts), meta, "node-tier")
}

async fn fanout_coordinator_tier(
    sql: &str,
    hierarchical_requested: bool,
) -> anyhow::Result<FanoutOutcome> {
    if hierarchical_requested {
        broadcast_fanout_query(sql, FanoutScope::Coordinator).await
    } else {
        fanout_flat(sql).await
    }
}

async fn fanout_flat(sql: &str) -> anyhow::Result<FanoutOutcome> {
    if can_fanout_via_global_catalog(sql) {
        fanout_via_global_catalog(sql, FanoutScope::Flat).await
    } else {
        broadcast_fanout_query(sql, FanoutScope::Flat).await
    }
}

async fn fanout_via_global_catalog(sql: &str, scope: FanoutScope) -> anyhow::Result<FanoutOutcome> {
    reset_fanout_stats();
    let global_sql = rewrite_sql_for_global_fanout(sql);
    log::debug!("cluster fan-out via global catalog ({scope:?}): {global_sql}");
    let dataframe = query_local_df_in_scope(scope, &global_sql).await?;
    let stats = take_fanout_stats();
    let nodes_queried = 1 + stats.nodes_succeeded + stats.nodes_failed.len();
    finish_fanout(
        dataframe,
        FanoutMeta {
            cluster: true,
            hierarchical: scope != FanoutScope::Flat,
            scope: scope.as_str().into(),
            nodes_queried,
            nodes_failed: stats.nodes_failed,
            peer_batches_dropped: stats.peer_batches_dropped,
            node_aggregators_queried: if scope == FanoutScope::Coordinator {
                nodes_queried.saturating_sub(1)
            } else {
                0
            },
            local_ranks_queried: if scope == FanoutScope::Node {
                nodes_queried.saturating_sub(1)
            } else {
                0
            },
            partial: false,
        },
        "global-catalog",
    )
}

async fn broadcast_fanout_query(sql: &str, scope: FanoutScope) -> anyhow::Result<FanoutOutcome> {
    if scope == FanoutScope::Coordinator && is_local0_from_env() {
        return broadcast_from_coordinator(sql).await;
    }
    broadcast_from_current_rank(sql, scope).await
}

async fn broadcast_from_coordinator(sql: &str) -> anyhow::Result<FanoutOutcome> {
    let mut parts = Vec::new();
    let mut meta = FanoutMeta::empty(true, true, ClusterFanoutScope::Coordinator);

    let local_outcome = fanout_node_tier(sql, true).await?;
    meta.local_ranks_queried = local_outcome.meta.local_ranks_queried;
    meta.absorb(local_outcome.meta);
    if !local_outcome.dataframe.is_empty() {
        parts.push(local_outcome.dataframe);
    }

    let node_aggregators = node_aggregator_peers();
    meta.node_aggregators_queried = node_aggregators.len();
    let client = HttpPeerQueryClient;
    for (node, result) in query_node_peers(&client, node_aggregators, sql).await {
        match result {
            Ok(outcome) => {
                meta.absorb(outcome.meta);
                if !outcome.dataframe.is_empty() {
                    parts.push(outcome.dataframe);
                }
            }
            Err(error) => {
                log::warn!("node aggregator fan-out {} failed: {error:#}", node.addr);
                meta.record_peer_failure(&node.addr, &error);
            }
        }
    }

    finish_fanout(
        merge_tagged_dataframes(&parts),
        meta,
        "coordinator-broadcast",
    )
}

async fn broadcast_from_current_rank(
    sql: &str,
    scope: FanoutScope,
) -> anyhow::Result<FanoutOutcome> {
    let host = local_host_label();
    let addr = probing_core::core::cluster::local_addr_label();
    let rank = cluster_rank_for_endpoint(&host, &addr);
    let mut parts = vec![tag_dataframe(
        query_local_df_in_scope(FanoutScope::Local, sql).await?,
        &host,
        &addr,
        rank,
    )];
    let mut meta = FanoutMeta::local(true, scope != FanoutScope::Flat, ClusterFanoutScope::Local);
    meta.scope = scope.as_str().into();
    let peers = peers_for_scope(scope);
    let peer_count = peers.len();
    let client = HttpPeerQueryClient;

    if scope == FanoutScope::Coordinator {
        meta.node_aggregators_queried = peer_count;
        for (node, result) in query_node_peers(&client, peers, sql).await {
            match result {
                Ok(outcome) => {
                    meta.absorb(outcome.meta);
                    if !outcome.dataframe.is_empty() {
                        parts.push(outcome.dataframe);
                    }
                }
                Err(error) => {
                    log::warn!("cluster fan-out {} failed: {error:#}", node.addr);
                    meta.record_peer_failure(&node.addr, &error);
                }
            }
        }
    } else {
        if scope == FanoutScope::Node {
            meta.local_ranks_queried = peer_count;
        }
        for (node, result) in query_leaf_peers(&client, peers, sql).await {
            match result {
                Ok(dataframe) => {
                    meta.record_peer_success();
                    parts.push(tag_dataframe(
                        dataframe,
                        if node.host.is_empty() {
                            &node.addr
                        } else {
                            &node.host
                        },
                        &node.addr,
                        node.rank,
                    ));
                }
                Err(error) => {
                    log::warn!("cluster fan-out {} failed: {error:#}", node.addr);
                    meta.record_peer_failure(&node.addr, &error);
                }
            }
        }
    }

    finish_fanout(merge_tagged_dataframes(&parts), meta, "broadcast")
}
