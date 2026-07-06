//! On-demand SQL fan-out across cluster nodes.
//!
//! Training agents write locally; cross-node aggregation runs only when a control-plane
//! caller explicitly requests `cluster=true`.
//!
//! **Hierarchical mode** (default when ``PROBING_CLUSTER_FANOUT_HIERARCHICAL`` is on):
//!
//! ```text
//! coordinator (rank0) ──► node aggregators (local0 per machine)
//!       each local0 ──► on-node leaf ranks ──► merge ──► coordinator
//! ```
//!
//! Single-table queries route through the `global` catalog (DataFusion federation).
//! JOIN / multi-statement SQL uses the legacy per-node broadcast path.

use probing_core::core::cluster::{
    hierarchical_metadata_available, hierarchical_metadata_unavailable_err, local_leaf_peers,
    local_listen_addrs, node_aggregator_peers,
};
use probing_core::core::federation::{
    can_fanout_via_global_catalog, cluster_rank_for_endpoint, is_local0_from_env,
    remote_fanout_concurrency, remote_query_timeout, reset_fanout_stats,
    rewrite_sql_for_global_fanout, take_fanout_stats, validate_global_query, with_fanout_scope,
    FanoutScope,
};
use probing_proto::prelude::*;

use crate::engine::handle_query;

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
        QueryDataFormat::DataFrame(df) => Ok(df),
        QueryDataFormat::Nil => Ok(DataFrame {
            names: vec![],
            cols: vec![],
            size: 0,
        }),
        QueryDataFormat::Error(err) => anyhow::bail!("query error: {}", err.message),
        QueryDataFormat::TimeSeries(_) => anyhow::bail!("unexpected timeseries"),
    }
}

pub async fn remote_query_df(addr: &str, sql: &str) -> anyhow::Result<DataFrame> {
    let url = format!("http://{addr}/query");
    let request = Message::new(Query {
        expr: sql.to_string(),
        ..Default::default()
    });
    let body = serde_json::to_string(&request)?;
    let timeout = remote_query_timeout();
    let response = tokio::task::spawn_blocking(move || {
        ureq::post(&url)
            .config()
            .timeout_global(Some(timeout))
            .build()
            .send(body)
            .map_err(anyhow::Error::new)
    })
    .await??;

    let status = response.status().as_u16();
    let text = response.into_body().read_to_string()?;
    if status >= 400 {
        anyhow::bail!("HTTP {status}: {text}");
    }

    let msg: Message<QueryDataFormat> = serde_json::from_str(&text)?;
    match msg.payload {
        QueryDataFormat::DataFrame(df) => Ok(df),
        QueryDataFormat::Nil => Ok(DataFrame {
            names: vec![],
            cols: vec![],
            size: 0,
        }),
        QueryDataFormat::Error(err) => anyhow::bail!("remote query: {}", err.message),
        QueryDataFormat::TimeSeries(_) => anyhow::bail!("unexpected timeseries"),
    }
}

async fn remote_node_aggregate_df(addr: &str, sql: &str) -> anyhow::Result<DataFrame> {
    let url = format!("http://{addr}/apis/cluster/query");
    let body = serde_json::json!({
        "expr": sql,
        "cluster": true,
        "hierarchical": true,
        "scope": "node",
    });
    let body = serde_json::to_string(&body)?;
    let timeout = remote_query_timeout();
    let response = tokio::task::spawn_blocking(move || {
        ureq::post(&url)
            .header("Content-Type", "application/json")
            .config()
            .timeout_global(Some(timeout))
            .build()
            .send(body)
            .map_err(anyhow::Error::new)
    })
    .await??;

    let status = response.status().as_u16();
    let text = response.into_body().read_to_string()?;
    if status >= 400 {
        anyhow::bail!("HTTP {status}: {text}");
    }

    let value: serde_json::Value = serde_json::from_str(&text)?;
    if let Some(err) = value.get("error").and_then(|v| v.as_str()) {
        anyhow::bail!("remote node aggregate: {err}");
    }
    let df = value
        .get("dataframe")
        .ok_or_else(|| anyhow::anyhow!("missing dataframe in node aggregate response"))?;
    Ok(serde_json::from_value(df.clone())?)
}

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
    fn resolve(self, hierarchical: bool) -> ClusterFanoutScope {
        match self {
            ClusterFanoutScope::Auto if hierarchical && is_local0_from_env() => {
                ClusterFanoutScope::Coordinator
            }
            ClusterFanoutScope::Auto if hierarchical => ClusterFanoutScope::Local,
            // Flat mode: coordinator fans out to all peers; leaf ranks stay local-only.
            ClusterFanoutScope::Auto if is_local0_from_env() => ClusterFanoutScope::Coordinator,
            ClusterFanoutScope::Auto => ClusterFanoutScope::Local,
            other => other,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Coordinator => "coordinator",
            Self::Node => "node",
            Self::Local => "local",
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct FanoutMeta {
    pub cluster: bool,
    pub hierarchical: bool,
    pub scope: String,
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
    fn finalize(&mut self) {
        self.partial = !self.nodes_failed.is_empty() || self.peer_batches_dropped > 0;
    }
}

fn finish_fanout(dataframe: DataFrame, mut meta: FanoutMeta, context: &str) -> FanoutQueryResponse {
    meta.finalize();
    if meta.partial {
        log::warn!(
            "cluster fan-out partial ({context}): nodes_queried={} nodes_failed={} peer_batches_dropped={}",
            meta.nodes_queried,
            meta.nodes_failed.len(),
            meta.peer_batches_dropped,
        );
    }
    FanoutQueryResponse { dataframe, meta }
}

async fn fanout_remote_plain(
    peers: Vec<Node>,
    sql: &str,
) -> Vec<(Node, anyhow::Result<DataFrame>)> {
    use futures_util::stream::{self, StreamExt};
    let sql = sql.to_string();
    let concurrency = remote_fanout_concurrency();
    stream::iter(peers)
        .map(|node| {
            let sql = sql.clone();
            async move {
                let result = remote_query_df(&node.addr, &sql).await;
                (node, result)
            }
        })
        .buffer_unordered(concurrency)
        .collect()
        .await
}

async fn fanout_remote_aggregate(
    peers: Vec<Node>,
    sql: &str,
) -> Vec<(Node, anyhow::Result<DataFrame>)> {
    use futures_util::stream::{self, StreamExt};
    let sql = sql.to_string();
    let concurrency = remote_fanout_concurrency();
    stream::iter(peers)
        .map(|node| {
            let sql = sql.clone();
            async move {
                let result = remote_node_aggregate_df(&node.addr, &sql).await;
                (node, result)
            }
        })
        .buffer_unordered(concurrency)
        .collect()
        .await
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct FanoutQueryResponse {
    pub dataframe: DataFrame,
    pub meta: FanoutMeta,
}

fn hierarchical_fanout_requested(hierarchical: bool) -> bool {
    hierarchical && probing_core::core::federation::hierarchical_fanout_enabled()
}

fn require_hierarchical_metadata() -> anyhow::Result<()> {
    if hierarchical_metadata_available() {
        Ok(())
    } else {
        Err(hierarchical_metadata_unavailable_err())
    }
}

/// Run `sql` locally, optionally fanning out to peer nodes in the cluster view.
pub async fn fanout_query(
    sql: &str,
    cluster: bool,
    hierarchical: bool,
    scope: ClusterFanoutScope,
) -> anyhow::Result<FanoutQueryResponse> {
    if cluster {
        validate_global_query(sql)?;
    }
    if !cluster {
        return Ok(FanoutQueryResponse {
            dataframe: query_local_df(sql).await?,
            meta: FanoutMeta {
                cluster: false,
                hierarchical,
                scope: ClusterFanoutScope::Local.as_str().into(),
                nodes_queried: 1,
                nodes_failed: Vec::new(),
                peer_batches_dropped: 0,
                node_aggregators_queried: 0,
                local_ranks_queried: 0,
                partial: false,
            },
        });
    }

    let hierarchical_requested = hierarchical_fanout_requested(hierarchical);
    let resolved_scope = scope.resolve(hierarchical_requested);

    if hierarchical_requested {
        match resolved_scope {
            ClusterFanoutScope::Coordinator | ClusterFanoutScope::Node if is_local0_from_env() => {
                require_hierarchical_metadata()?;
            }
            _ => {}
        }
    }

    match resolved_scope {
        ClusterFanoutScope::Local => {
            let dataframe = query_local_df(sql).await?;
            Ok(finish_fanout(
                dataframe,
                FanoutMeta {
                    cluster: true,
                    hierarchical,
                    scope: resolved_scope.as_str().into(),
                    nodes_queried: 1,
                    nodes_failed: Vec::new(),
                    peer_batches_dropped: 0,
                    node_aggregators_queried: 0,
                    local_ranks_queried: 0,
                    partial: false,
                },
                "local",
            ))
        }
        ClusterFanoutScope::Node => fanout_node_tier(sql, hierarchical).await,
        ClusterFanoutScope::Coordinator => fanout_coordinator_tier(sql, hierarchical).await,
        ClusterFanoutScope::Auto => unreachable!("scope::Auto must be resolved"),
    }
}

/// Node aggregator: local0 + on-node leaf ranks.
async fn fanout_node_tier(sql: &str, hierarchical: bool) -> anyhow::Result<FanoutQueryResponse> {
    if !is_local0_from_env() {
        let dataframe = query_local_df(sql).await?;
        return Ok(finish_fanout(
            dataframe,
            FanoutMeta {
                cluster: true,
                hierarchical,
                scope: ClusterFanoutScope::Local.as_str().into(),
                nodes_queried: 1,
                nodes_failed: Vec::new(),
                peer_batches_dropped: 0,
                node_aggregators_queried: 0,
                local_ranks_queried: 0,
                partial: false,
            },
            "node-tier-local-fallback",
        ));
    }

    let host = local_host_label();
    let addr = probing_core::core::cluster::local_addr_label();
    let local_rank = cluster_rank_for_endpoint(&host, &addr);

    let mut nodes_failed = Vec::new();
    let mut parts = Vec::new();

    let local_df = with_fanout_scope(FanoutScope::Node, || {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(query_local_df(sql))
        })
    })?;
    parts.push(tag_dataframe(local_df, &host, &addr, local_rank));

    let leaves = local_leaf_peers();
    let local_ranks_queried = leaves.len();
    let mut nodes_queried = 1usize;
    let responses = fanout_remote_plain(leaves, sql).await;

    for (node, result) in responses {
        match result {
            Ok(df) => {
                parts.push(tag_dataframe(
                    df,
                    if node.host.is_empty() {
                        &node.addr
                    } else {
                        &node.host
                    },
                    &node.addr,
                    node.rank,
                ));
                nodes_queried += 1;
            }
            Err(err) => {
                log::warn!("local leaf fan-out {} failed: {err}", node.addr);
                nodes_failed.push(node.addr);
            }
        }
    }

    Ok(finish_fanout(
        merge_tagged_dataframes(&parts),
        FanoutMeta {
            cluster: true,
            hierarchical,
            scope: ClusterFanoutScope::Node.as_str().into(),
            nodes_queried,
            nodes_failed,
            peer_batches_dropped: 0,
            node_aggregators_queried: 0,
            local_ranks_queried,
            partial: false,
        },
        "node-tier",
    ))
}

/// Global coordinator: node aggregators (+ on-node leaves via broadcast path).
async fn fanout_coordinator_tier(
    sql: &str,
    hierarchical: bool,
) -> anyhow::Result<FanoutQueryResponse> {
    if hierarchical_fanout_requested(hierarchical) {
        require_hierarchical_metadata()?;
        return broadcast_fanout_query(sql, FanoutScope::Coordinator).await;
    }
    fanout_flat(sql).await
}

/// Legacy flat fan-out to every registered peer.
async fn fanout_flat(sql: &str) -> anyhow::Result<FanoutQueryResponse> {
    if can_fanout_via_global_catalog(sql) {
        return fanout_via_global_catalog(sql, FanoutScope::Flat).await;
    }
    broadcast_fanout_query(sql, FanoutScope::Flat).await
}

async fn fanout_via_global_catalog(
    sql: &str,
    scope: FanoutScope,
) -> anyhow::Result<FanoutQueryResponse> {
    reset_fanout_stats();
    let global_sql = rewrite_sql_for_global_fanout(sql);
    log::debug!("cluster fan-out via global catalog ({scope:?}): {global_sql}");
    let dataframe = with_fanout_scope(scope, || {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(query_local_df(&global_sql))
        })
    })?;
    let stats = take_fanout_stats();
    Ok(finish_fanout(
        dataframe,
        FanoutMeta {
            cluster: true,
            hierarchical: scope != FanoutScope::Flat,
            scope: scope.as_str().into(),
            nodes_queried: 1 + stats.nodes_succeeded,
            nodes_failed: stats.nodes_failed,
            peer_batches_dropped: stats.peer_batches_dropped,
            node_aggregators_queried: if scope == FanoutScope::Coordinator {
                stats.nodes_succeeded
            } else {
                0
            },
            local_ranks_queried: if scope == FanoutScope::Node {
                stats.nodes_succeeded
            } else {
                0
            },
            partial: false,
        },
        "global-catalog",
    ))
}

async fn broadcast_fanout_query(
    sql: &str,
    scope: FanoutScope,
) -> anyhow::Result<FanoutQueryResponse> {
    if scope == FanoutScope::Coordinator && is_local0_from_env() {
        let mut parts = Vec::new();
        let mut meta = FanoutMeta {
            cluster: true,
            hierarchical: true,
            scope: scope.as_str().into(),
            nodes_queried: 0,
            nodes_failed: Vec::new(),
            peer_batches_dropped: 0,
            node_aggregators_queried: 0,
            local_ranks_queried: 0,
            partial: false,
        };

        let node_part = fanout_node_tier(sql, true).await?;
        meta.local_ranks_queried = node_part.meta.local_ranks_queried;
        meta.nodes_queried += node_part.meta.nodes_queried;
        meta.nodes_failed.extend(node_part.meta.nodes_failed);
        meta.peer_batches_dropped += node_part.meta.peer_batches_dropped;
        if !node_part.dataframe.is_empty() {
            parts.push(node_part.dataframe);
        }

        let node_aggs = node_aggregator_peers();
        meta.node_aggregators_queried = node_aggs.len();
        let responses = fanout_remote_aggregate(node_aggs, sql).await;

        for (node, result) in responses {
            match result {
                Ok(df) => {
                    parts.push(tag_dataframe(
                        df,
                        if node.host.is_empty() {
                            &node.addr
                        } else {
                            &node.host
                        },
                        &node.addr,
                        node.rank,
                    ));
                    meta.nodes_queried += 1;
                }
                Err(err) => {
                    log::warn!("node aggregator fan-out {} failed: {err}", node.addr);
                    meta.nodes_failed.push(node.addr);
                }
            }
        }

        return Ok(finish_fanout(
            merge_tagged_dataframes(&parts),
            meta,
            "coordinator-broadcast",
        ));
    }

    let host = local_host_label();
    let addr = probing_core::core::cluster::local_addr_label();
    let local_rank = cluster_rank_for_endpoint(&host, &addr);
    let mut parts = vec![tag_dataframe(
        with_fanout_scope(FanoutScope::Local, || {
            tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(query_local_df(sql))
            })
        })?,
        &host,
        &addr,
        local_rank,
    )];
    let mut nodes_queried = 1usize;
    let mut nodes_failed = Vec::new();

    let peers = match scope {
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
    };

    let peer_count = peers.len();
    let responses = if scope == FanoutScope::Coordinator {
        fanout_remote_aggregate(peers, sql).await
    } else {
        fanout_remote_plain(peers, sql).await
    };

    for (node, result) in responses {
        match result {
            Ok(df) => {
                parts.push(tag_dataframe(
                    df,
                    if node.host.is_empty() {
                        &node.addr
                    } else {
                        &node.host
                    },
                    &node.addr,
                    node.rank,
                ));
                nodes_queried += 1;
            }
            Err(err) => {
                log::warn!("cluster fan-out {} failed: {err}", node.addr);
                nodes_failed.push(node.addr);
            }
        }
    }

    Ok(finish_fanout(
        merge_tagged_dataframes(&parts),
        FanoutMeta {
            cluster: true,
            hierarchical: scope != FanoutScope::Flat,
            scope: scope.as_str().into(),
            nodes_queried,
            nodes_failed,
            peer_batches_dropped: 0,
            node_aggregators_queried: if scope == FanoutScope::Coordinator {
                peer_count
            } else {
                0
            },
            local_ranks_queried: if scope == FanoutScope::Node {
                peer_count
            } else {
                0
            },
            partial: false,
        },
        "broadcast",
    ))
}

fn tag_dataframe(mut df: DataFrame, host: &str, addr: &str, rank: Option<i32>) -> DataFrame {
    if df.is_empty() {
        return df;
    }
    probing_core::core::federation::tag_proto_dataframe(&mut df, host, addr, rank);
    df
}

fn merge_tagged_dataframes(parts: &[DataFrame]) -> DataFrame {
    probing_proto::types::merge_dataframes(parts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_preserves_probe_tags() {
        let local = tag_dataframe(
            DataFrame {
                names: vec!["rank".into()],
                cols: vec![Seq::SeqI32(vec![0])],
                size: 1,
            },
            "host-a",
            "10.0.0.1:8080",
            Some(0),
        );
        let remote = tag_dataframe(
            DataFrame {
                names: vec!["rank".into()],
                cols: vec![Seq::SeqI32(vec![1])],
                size: 1,
            },
            "host-b",
            "10.0.0.2:8080",
            Some(1),
        );
        let merged = merge_tagged_dataframes(&[local, remote]);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged.names.len(), 7);
        let host_col = merged.names.iter().position(|n| n == "_host").unwrap();
        assert_eq!(merged.cols[host_col].get_str(0).as_deref(), Some("host-a"));
        assert_eq!(merged.cols[host_col].get_str(1).as_deref(), Some("host-b"));
    }

    #[test]
    fn merge_aligns_missing_columns_with_empty_strings() {
        let a = DataFrame {
            names: vec!["x".into(), "extra".into()],
            cols: vec![Seq::SeqI32(vec![1]), Seq::SeqText(vec!["a".into()])],
            size: 1,
        };
        let b = DataFrame {
            names: vec!["x".into()],
            cols: vec![Seq::SeqI32(vec![2])],
            size: 1,
        };
        let merged = merge_tagged_dataframes(&[a, b]);
        assert_eq!(merged.len(), 2);
        assert!(merged.names.contains(&"extra".to_string()));
    }

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
        clean.finalize();
        assert!(!clean.partial);

        clean.peer_batches_dropped = 2;
        clean.finalize();
        assert!(clean.partial);
    }
}
