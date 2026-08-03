use std::collections::BTreeSet;
use std::time::{Duration, SystemTime};

use chrono::{DateTime, Utc};
use dioxus::prelude::*;
use probing_proto::prelude::Node;

use crate::api::ApiClient;

use super::super::components::{
    EvidenceMetric, LoadingPanel, SectionCard, UnavailablePanel, WorkspacePage,
};
use super::super::settings::DISTRIBUTED_REFRESH;

#[component]
pub fn ClusterPage() -> Element {
    let nodes = use_resource(|| {
        let _ = *DISTRIBUTED_REFRESH.read();
        async move { ApiClient::new().get_nodes().await }
    });
    let state = nodes.read().clone();

    rsx! {
        WorkspacePage {
            title: "Cluster Nodes".to_string(),
            subtitle: "Heartbeat registry entries with physical rank placement and the values reported by each process.".to_string(),

            match state {
                None => rsx! { LoadingPanel { label: "Loading node registry".to_string() } },
                Some(Err(error)) => rsx! { UnavailablePanel {
                    label: "Node registry unavailable".to_string(),
                    detail: error.display_message(),
                }},
                Some(Ok(nodes)) if nodes.is_empty() => rsx! { UnavailablePanel {
                    label: "No nodes registered".to_string(),
                    detail: "The heartbeat registry returned zero entries.".to_string(),
                }},
                Some(Ok(nodes)) => rsx! { ClusterRegistry { nodes } },
            }
        }
    }
}

#[component]
fn ClusterRegistry(nodes: Vec<Node>) -> Element {
    let summary = RegistrySummary::from_nodes(&nodes);
    let mut rows = nodes;
    rows.sort_by(|left, right| {
        left.rank
            .unwrap_or(i32::MAX)
            .cmp(&right.rank.unwrap_or(i32::MAX))
            .then_with(|| left.host.cmp(&right.host))
            .then_with(|| left.local_rank.cmp(&right.local_rank))
    });
    let now_micros = unix_time_micros();

    rsx! {
        SectionCard {
            title: "Registry summary".to_string(),
            subtitle: Some("Counts are derived directly from the current heartbeat snapshot.".to_string()),
            RegistrySummaryStrip { summary }
        }
        SectionCard {
            title: "Registered processes".to_string(),
            subtitle: Some("One row per reported process; rank and placement fields are not inferred when absent.".to_string()),
            body_class: "p-0".to_string(),
            NodeTable { nodes: rows, now_micros }
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct RegistrySummary {
    processes: usize,
    hosts: usize,
    observed_ranks: usize,
    expected_ranks: Option<usize>,
    world_sizes: Vec<i32>,
}

impl RegistrySummary {
    fn from_nodes(nodes: &[Node]) -> Self {
        let hosts = nodes
            .iter()
            .map(|node| node.host.as_str())
            .collect::<BTreeSet<_>>()
            .len();
        let observed_ranks = nodes
            .iter()
            .filter_map(|node| node.rank)
            .collect::<BTreeSet<_>>()
            .len();
        let world_sizes = nodes
            .iter()
            .filter_map(|node| node.world_size)
            .filter(|world_size| *world_size > 0)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let expected_ranks = (world_sizes.len() == 1).then_some(world_sizes[0] as usize);

        Self {
            processes: nodes.len(),
            hosts,
            observed_ranks,
            expected_ranks,
            world_sizes,
        }
    }

    fn rank_coverage(&self) -> String {
        self.expected_ranks
            .map(|expected| format!("{} / {expected}", self.observed_ranks))
            .unwrap_or_else(|| self.observed_ranks.to_string())
    }

    fn world_size_label(&self) -> String {
        if self.world_sizes.is_empty() {
            "—".to_string()
        } else {
            self.world_sizes
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        }
    }
}

#[component]
fn RegistrySummaryStrip(summary: RegistrySummary) -> Element {
    rsx! {
        div { class: "grid grid-cols-4 divide-x divide-gray-200",
            EvidenceMetric { label: "Processes", value: summary.processes.to_string(), detail: "registry rows".to_string() }
            EvidenceMetric { label: "Hosts", value: summary.hosts.to_string(), detail: "unique host names".to_string() }
            EvidenceMetric { label: "Ranks", value: summary.rank_coverage(), detail: "observed / reported world".to_string() }
            EvidenceMetric { label: "World size", value: summary.world_size_label(), detail: "distinct reported values".to_string() }
        }
    }
}

#[component]
fn NodeTable(nodes: Vec<Node>, now_micros: u64) -> Element {
    rsx! {
        div { class: "overflow-x-auto",
            table { class: "w-full border-collapse text-xs",
                thead {
                    tr { class: "border-b border-gray-200 bg-gray-50 text-left text-xs uppercase tracking-wide text-gray-500",
                        th { class: "px-3 py-2 font-medium", "Host" }
                        th { class: "px-3 py-2 font-medium", "Endpoint" }
                        th { class: "px-3 py-2 text-right font-medium", "Rank" }
                        th { class: "px-3 py-2 font-medium", "Physical placement" }
                        th { class: "px-3 py-2 font-medium", "Parallel coordinates" }
                        th { class: "px-3 py-2 font-medium", "Role" }
                        th { class: "px-3 py-2 font-medium", "State" }
                        th { class: "px-3 py-2 text-right font-medium", "Heartbeat" }
                    }
                }
                tbody { class: "divide-y divide-gray-100",
                    for node in nodes {
                        NodeRow { node, now_micros }
                    }
                }
            }
        }
    }
}

#[component]
fn NodeRow(node: Node, now_micros: u64) -> Element {
    let endpoint = format!("http://{}", node.addr);
    let rank = node
        .rank
        .map(|rank| rank.to_string())
        .unwrap_or_else(|| "—".to_string());
    let world = node
        .world_size
        .map(|world| format!(" / {world}"))
        .unwrap_or_default();
    let node_rank = node
        .group_rank
        .map(|rank| rank.to_string())
        .unwrap_or_else(|| "—".to_string());
    let local_rank = node
        .local_rank
        .map(|rank| rank.to_string())
        .unwrap_or_else(|| "—".to_string());
    let coordinates = node.role.clone().unwrap_or_else(|| "—".to_string());
    let role = node.role_name.clone().unwrap_or_else(|| "—".to_string());
    let state = node
        .status
        .clone()
        .filter(|status| !status.trim().is_empty())
        .unwrap_or_else(|| "not reported".to_string());
    let state_class = status_classes(node.status.as_deref());
    let heartbeat = format_heartbeat_age(now_micros, node.timestamp);
    let heartbeat_title = format_heartbeat_timestamp(node.timestamp);

    rsx! {
        tr { class: "hover:bg-gray-50/70",
            td { class: "px-3 py-2 font-medium text-gray-900", "{node.host}" }
            td { class: "px-3 py-2",
                a {
                    href: endpoint,
                    target: "_blank",
                    class: "font-mono text-xs text-blue-700 hover:underline",
                    "{node.addr}"
                }
            }
            td { class: "px-3 py-2 text-right font-mono tabular-nums text-gray-700",
                "{rank}"
                span { class: "text-gray-500", "{world}" }
            }
            td { class: "px-3 py-2 font-mono text-xs text-gray-600",
                "node {node_rank} · GPU {local_rank}"
            }
            td { class: "max-w-52 break-all px-3 py-2 font-mono text-xs text-violet-700", "{coordinates}" }
            td { class: "px-3 py-2 text-gray-600", "{role}" }
            td { class: "px-3 py-2",
                span { class: "inline-flex rounded-full border px-2 py-0.5 text-xs font-medium {state_class}", "{state}" }
            }
            td { class: "px-3 py-2 text-right font-mono text-xs text-gray-600",
                div { class: "whitespace-nowrap", "{heartbeat}" }
                div { class: "mt-0.5 whitespace-nowrap text-gray-500", "{heartbeat_title}" }
            }
        }
    }
}

fn status_classes(status: Option<&str>) -> &'static str {
    match status.unwrap_or("").trim().to_ascii_lowercase().as_str() {
        "ok" | "healthy" | "running" | "ready" | "online" => {
            "border-emerald-200 bg-emerald-50 text-emerald-700"
        }
        "failed" | "error" | "offline" | "unhealthy" => "border-red-200 bg-red-50 text-red-700",
        "" => "border-gray-200 bg-gray-100 text-gray-600",
        _ => "border-amber-200 bg-amber-50 text-amber-700",
    }
}

fn unix_time_micros() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros()
        .min(u64::MAX as u128) as u64
}

fn format_heartbeat_age(now_micros: u64, timestamp: u64) -> String {
    if timestamp == 0 {
        return "—".to_string();
    }
    let seconds = now_micros.saturating_sub(timestamp) / 1_000_000;
    match seconds {
        0 => "<1s ago".to_string(),
        1..=59 => format!("{seconds}s ago"),
        60..=3_599 => format!("{}m ago", seconds / 60),
        3_600..=86_399 => format!("{}h ago", seconds / 3_600),
        _ => format!("{}d ago", seconds / 86_400),
    }
}

fn format_heartbeat_timestamp(timestamp: u64) -> String {
    if timestamp == 0 {
        return "timestamp not reported".to_string();
    }
    let instant = SystemTime::UNIX_EPOCH + Duration::from_micros(timestamp);
    let datetime: DateTime<Utc> = instant.into();
    datetime.format("%Y-%m-%d %H:%M:%S UTC").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(host: &str, rank: i32, world_size: i32) -> Node {
        Node {
            host: host.to_string(),
            rank: Some(rank),
            world_size: Some(world_size),
            ..Default::default()
        }
    }

    #[test]
    fn registry_summary_keeps_reported_counts_separate() {
        let nodes = vec![
            node("host-a", 0, 4),
            node("host-a", 1, 4),
            node("host-b", 3, 4),
        ];
        let summary = RegistrySummary::from_nodes(&nodes);

        assert_eq!(summary.processes, 3);
        assert_eq!(summary.hosts, 2);
        assert_eq!(summary.rank_coverage(), "3 / 4");
        assert_eq!(summary.world_size_label(), "4");
    }

    #[test]
    fn conflicting_world_sizes_remain_visible() {
        let nodes = vec![node("host-a", 0, 8), node("host-b", 1, 16)];
        let summary = RegistrySummary::from_nodes(&nodes);

        assert_eq!(summary.expected_ranks, None);
        assert_eq!(summary.rank_coverage(), "2");
        assert_eq!(summary.world_size_label(), "8, 16");
    }

    #[test]
    fn heartbeat_age_is_a_direct_time_delta() {
        assert_eq!(format_heartbeat_age(10_000_000, 8_500_000), "1s ago");
        assert_eq!(format_heartbeat_age(120_000_000, 0), "—");
    }
}
