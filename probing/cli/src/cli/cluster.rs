use anyhow::Result;
use probing_proto::prelude::NodeListResponse;

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
    if let Some(err) = value.get("error").and_then(|v| v.as_str()) {
        anyhow::bail!("{err}");
    }
    let df = value
        .get("dataframe")
        .ok_or_else(|| anyhow::anyhow!("missing dataframe in response"))?;
    let dataframe: probing_proto::prelude::DataFrame = serde_json::from_value(df.clone())?;
    if let Some(meta) = value.get("meta") {
        let nodes = meta
            .get("nodes_queried")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let failed = meta
            .get("nodes_failed")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        eprintln!("cluster query: cluster={cluster}, nodes_queried={nodes}, nodes_failed={failed}");
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
