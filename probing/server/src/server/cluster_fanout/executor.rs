use futures_util::stream::{self, StreamExt};
use probing_proto::prelude::{DataFrame, Node};

use super::transport::PeerQueryClient;
use super::types::FanoutOutcome;

pub(super) async fn query_leaf_peers<C: PeerQueryClient>(
    client: &C,
    peers: Vec<Node>,
    sql: &str,
) -> Vec<(Node, anyhow::Result<DataFrame>)> {
    let sql = sql.to_string();
    let request_count = peers.len().max(1);
    stream::iter(peers)
        .map(|node| {
            let sql = sql.clone();
            async move {
                let result = client.query_leaf(&node.addr, &sql).await;
                (node, result)
            }
        })
        .buffer_unordered(request_count)
        .collect()
        .await
}

pub(super) async fn query_node_peers<C: PeerQueryClient>(
    client: &C,
    peers: Vec<Node>,
    sql: &str,
) -> Vec<(Node, anyhow::Result<FanoutOutcome>)> {
    let sql = sql.to_string();
    let request_count = peers.len().max(1);
    stream::iter(peers)
        .map(|node| {
            let sql = sql.clone();
            async move {
                let result = client.query_node(&node.addr, &sql).await;
                (node, result)
            }
        })
        .buffer_unordered(request_count)
        .collect()
        .await
}
