use anyhow::{Context, Result};

use super::vars::read_probing_address;
use crate::cluster_http::{get_i32_env, put_nodes_blocking};
use crate::cluster_report_backoff::{classify_report_outcome, ReportBackoff, ReportOutcome};
use crate::server::SERVER_RUNTIME;
use probing_proto::prelude::{Node, NodeReportResponse};

pub fn get_hostname() -> Result<String> {
    let uname = nix::sys::utsname::uname()?;
    let hostname = uname.nodename().to_string_lossy().to_string();
    Ok(hostname)
}

fn cluster_report_enabled() -> bool {
    match std::env::var("PROBING_CLUSTER_REPORT") {
        Ok(val) => {
            let lower = val.trim().to_ascii_lowercase();
            !matches!(lower.as_str(), "0" | "false" | "no" | "off")
        }
        Err(_) => true,
    }
}

pub fn start_report_worker(report_addr: String, local_addr: String) {
    if !cluster_report_enabled() {
        log::info!("cluster report worker disabled (PROBING_CLUSTER_REPORT=0)");
        return;
    }
    if crate::torchrun_cluster::is_torchrun_cluster_active() {
        log::debug!("skip flat report worker: torchrun hierarchical cluster active");
        return;
    }
    log::debug!("start report worker: {local_addr} => {report_addr}");
    SERVER_RUNTIME.spawn(report_worker(report_addr, local_addr));
}

async fn report_worker(report_addr: String, local_addr: String) {
    let mut backoff = ReportBackoff::new();

    loop {
        let report_url = format!("http://{report_addr}/apis/nodes");
        let node = build_local_node(&local_addr);
        let rank = node.rank.unwrap_or(-1);

        let outcome = if rank == 0 && report_addr == local_addr {
            let node = node.clone();
            match tokio::task::spawn_blocking(move || {
                probing_core::core::cluster::apply_node_report(
                    vec![node],
                    probing_core::core::cluster::cluster_version(),
                )
            })
            .await
            {
                Ok(snapshot) => classify_report_outcome(true, true, Some(&snapshot)),
                Err(_) => ReportOutcome::Failed,
            }
        } else {
            match request_remote(&report_url, vec![node]).await {
                Ok(reply) => {
                    log::debug!(
                        "node status reported to {report_url}: version={}",
                        reply.version
                    );
                    classify_report_outcome(true, true, Some(&reply))
                }
                Err(err) => {
                    log::error!("failed to report to {report_url}: {err}");
                    ReportOutcome::Failed
                }
            }
        };

        let sleep_for = {
            backoff.record(outcome);
            backoff.sleep_duration()
        };
        tokio::time::sleep(sleep_for).await;
    }
}

pub(crate) fn build_local_node(local_addr: &str) -> Node {
    let hostname = get_hostname().unwrap_or_else(|_| "localhost".to_string());
    let address = {
        let probing_address = read_probing_address();
        if !probing_address.is_empty() {
            probing_address.clone()
        } else {
            local_addr.to_string()
        }
    };
    Node {
        host: hostname,
        addr: address,
        local_rank: get_i32_env("LOCAL_RANK"),
        rank: get_i32_env("RANK"),
        world_size: get_i32_env("WORLD_SIZE"),
        group_rank: get_i32_env("GROUP_RANK").or_else(|| get_i32_env("NODE_RANK")),
        group_world_size: get_i32_env("GROUP_WORLD_SIZE"),
        role_name: std::env::var("ROLE_NAME").ok(),
        role_rank: get_i32_env("ROLE_RANK"),
        role_world_size: get_i32_env("ROLE_WORLD_SIZE"),
        role: std::env::var("PROBING_NODE_ROLE")
            .ok()
            .filter(|s| !s.trim().is_empty()),
        status: Some("running".to_string()),
        timestamp: 0,
    }
}

async fn request_remote(url: &str, nodes: Vec<Node>) -> Result<NodeReportResponse> {
    let url = url.to_string();
    tokio::task::spawn_blocking(move || request_remote_blocking(&url, nodes))
        .await
        .context("cluster report spawn_blocking failed")?
}

fn request_remote_blocking(url: &str, nodes: Vec<Node>) -> Result<NodeReportResponse> {
    let base = url.strip_suffix("/apis/nodes").unwrap_or(url);
    put_nodes_blocking(base, nodes, probing_core::core::cluster::cluster_version())
}
