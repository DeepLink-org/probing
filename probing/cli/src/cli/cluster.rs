use anyhow::Result;
use probing_proto::prelude::NodeListResponse;
use probing_skills::backend::parse_cluster_query_response;

use crate::cli::ctrl::ProbeEndpoint;
use crate::table::render_dataframe;

#[derive(clap::Subcommand, Debug, Clone)]
pub enum ClusterCommand {
    /// Fan-out SQL across cluster nodes (on-demand; default queries all peers)
    Query {
        #[arg()]
        query: String,
        /// Query only the connected endpoint (skip cluster fan-out)
        #[arg(long)]
        local: bool,
        /// Flat fan-out to every registered peer (disable hierarchical aggregation)
        #[arg(long)]
        flat: bool,
    },
    /// List nodes in the cluster view
    Nodes,
}

pub async fn run(ctrl: ProbeEndpoint, cmd: ClusterCommand) -> Result<()> {
    match cmd {
        ClusterCommand::Query { query, local, flat } => {
            cluster_query(ctrl, &query, !local, !flat).await
        }
        ClusterCommand::Nodes => cluster_nodes(ctrl).await,
    }
}

async fn cluster_query(
    ctrl: ProbeEndpoint,
    expr: &str,
    cluster: bool,
    hierarchical: bool,
) -> Result<()> {
    let body = serde_json::json!({
        "expr": expr,
        "cluster": cluster,
        "hierarchical": hierarchical,
    });
    let reply = ctrl
        .post_json("/apis/cluster/query", &body.to_string())
        .await?;
    let value: serde_json::Value = serde_json::from_str(&reply)?;
    let (dataframe, cluster_meta) =
        parse_cluster_query_response(&value).map_err(|e| anyhow::anyhow!(e.0))?;
    if let Some(meta) = cluster_meta {
        eprintln!(
            "cluster query: cluster={cluster}, nodes_queried={}, nodes_failed={}",
            meta.nodes_queried,
            meta.nodes_failed.len()
        );
    }
    render_dataframe(&dataframe);
    Ok(())
}

async fn cluster_nodes(ctrl: ProbeEndpoint) -> Result<()> {
    let mut all = Vec::new();
    let mut offset = 0usize;
    loop {
        let reply = ctrl
            .get(&format!("/apis/nodes?offset={offset}&limit=1024"))
            .await?;
        let page: NodeListResponse = serde_json::from_str(&reply)?;
        let empty = page.nodes.is_empty();
        all.extend(page.nodes);
        if all.len() >= page.total || empty {
            break;
        }
        offset = offset.saturating_add(1024);
    }
    if all.is_empty() {
        println!("No cluster nodes registered.");
        return Ok(());
    }
    for node in all {
        println!(
            "{}:{} rank={:?} world_size={:?} status={:?}",
            node.host, node.addr, node.rank, node.world_size, node.status
        );
    }
    Ok(())
}
