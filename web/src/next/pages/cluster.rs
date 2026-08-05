use chrono::{DateTime, Utc};
use dioxus::prelude::*;
use probing_proto::prelude::Node;

use crate::api::ApiClient;
use crate::state::investigation::{set_node_context, InvestigationContext, INVESTIGATION_CONTEXT};

use super::super::components::{
    EvidenceMetric, FilterInput, LoadingPanel, SectionCard, UnavailablePanel, WorkspacePage,
};
use super::super::model::RegistryHealth;
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
                    label: "Heartbeat registry unavailable".to_string(),
                    detail: error.display_message(),
                }},
                Some(Ok(nodes)) if nodes.is_empty() => rsx! { UnavailablePanel {
                    label: "No processes registered".to_string(),
                    detail: "The heartbeat registry returned zero entries.".to_string(),
                }},
                Some(Ok(nodes)) => rsx! { ClusterRegistry { nodes } },
            }
        }
    }
}

#[component]
fn ClusterRegistry(nodes: Vec<Node>) -> Element {
    let mut filter = use_signal(String::new);
    let mut pinned_only = use_signal(|| false);
    let summary = RegistryHealth::from_nodes(&nodes);
    let mut rows = nodes;
    rows.sort_by(|left, right| {
        left.rank
            .unwrap_or(i32::MAX)
            .cmp(&right.rank.unwrap_or(i32::MAX))
            .then_with(|| left.host.cmp(&right.host))
            .then_with(|| left.local_rank.cmp(&right.local_rank))
    });
    let context = INVESTIGATION_CONTEXT.read().clone();
    let has_pinned_node = has_node_coordinates(&context);
    let filter_value = filter();
    let query = filter_value.trim().to_ascii_lowercase();
    let show_pinned_only = pinned_only() && has_pinned_node;
    let filtered_rows = rows
        .iter()
        .filter(|node| {
            (!show_pinned_only || node_matches_context(node, &context))
                && node_matches_query(node, &query)
        })
        .cloned()
        .collect::<Vec<_>>();
    let shown = filtered_rows.len();
    let total = rows.len();
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
            div { class: "flex flex-wrap items-center gap-3 border-b border-gray-200 bg-gray-50/70 px-3 py-2",
                FilterInput {
                    value: filter_value,
                    placeholder: "Filter rank, host, GPU, endpoint, or role".to_string(),
                    class: "min-w-72 flex-1".to_string(),
                    oninput: move |value| filter.set(value),
                }
                label { class: if has_pinned_node { "flex cursor-pointer items-center gap-2 text-xs text-gray-700" } else { "flex cursor-not-allowed items-center gap-2 text-xs text-gray-400" },
                    input {
                        r#type: "checkbox",
                        checked: show_pinned_only,
                        disabled: !has_pinned_node,
                        class: "h-4 w-4 rounded border-gray-300 text-blue-600 focus:ring-blue-500",
                        onchange: move |_| pinned_only.set(!pinned_only()),
                    }
                    "Pinned process only"
                }
                span { class: "text-xs tabular-nums text-gray-500", "{shown} / {total} rows" }
            }
            if filtered_rows.is_empty() {
                div { class: "p-3",
                    UnavailablePanel {
                        label: "No matching registered processes".to_string(),
                        detail: "The current registry snapshot contains no row matching the active filters.".to_string(),
                    }
                }
            } else {
                NodeTable { nodes: filtered_rows, now_micros }
            }
        }
    }
}

#[component]
fn RegistrySummaryStrip(summary: RegistryHealth) -> Element {
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
                        th { class: "px-3 py-2 text-right font-medium", "Evidence" }
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
    let selected = node_matches_context(&node, &INVESTIGATION_CONTEXT.read().clone());
    let context_host = node.host.clone();
    let context_rank = node.rank;
    let context_device = node.local_rank;
    let select_label = format!("Select rank {} on {} GPU {}", rank, node.host, local_rank);

    rsx! {
        tr { class: if selected { "bg-blue-50/60 ring-1 ring-inset ring-blue-100" } else { "hover:bg-gray-50/70" },
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
            td { class: "px-3 py-2 text-right",
                button {
                    r#type: "button",
                    class: if selected {
                        "rounded-md bg-blue-100 px-2 py-1 text-xs font-medium text-blue-800"
                    } else {
                        "rounded-md border border-gray-200 bg-white px-2 py-1 text-xs font-medium text-blue-700 hover:border-blue-300 hover:bg-blue-50"
                    },
                    aria_pressed: selected.to_string(),
                    aria_label: select_label,
                    onclick: move |_| set_node_context(context_rank, Some(&context_host), context_device),
                    if selected { "Selected" } else { "Select" }
                }
            }
        }
    }
}

fn has_node_coordinates(context: &InvestigationContext) -> bool {
    context.rank.is_some() || context.host.is_some() || context.device_id.is_some()
}

fn node_matches_context(node: &Node, context: &InvestigationContext) -> bool {
    has_node_coordinates(context)
        && context.rank.is_none_or(|rank| node.rank == Some(rank))
        && context.host.as_ref().is_none_or(|host| node.host == *host)
        && context
            .device_id
            .is_none_or(|device| node.local_rank == Some(device))
}

fn node_matches_query(node: &Node, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let fields = [
        node.host.clone(),
        node.addr.clone(),
        node.rank.map(|value| value.to_string()).unwrap_or_default(),
        node.local_rank
            .map(|value| format!("gpu {value}"))
            .unwrap_or_default(),
        node.role.clone().unwrap_or_default(),
        node.role_name.clone().unwrap_or_default(),
        node.status.clone().unwrap_or_default(),
    ];
    fields
        .iter()
        .any(|field| field.to_ascii_lowercase().contains(query))
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

#[cfg(target_arch = "wasm32")]
fn unix_time_micros() -> u64 {
    (js_sys::Date::now() * 1_000.0).max(0.0) as u64
}

#[cfg(not(target_arch = "wasm32"))]
fn unix_time_micros() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
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
    i64::try_from(timestamp)
        .ok()
        .and_then(DateTime::<Utc>::from_timestamp_micros)
        .map(|datetime| datetime.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .unwrap_or_else(|| "invalid timestamp".to_string())
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
        let summary = RegistryHealth::from_nodes(&nodes);

        assert_eq!(summary.processes, 3);
        assert_eq!(summary.hosts, 2);
        assert_eq!(summary.rank_coverage(), "3 / 4");
        assert_eq!(summary.world_size_label(), "4");
    }

    #[test]
    fn conflicting_world_sizes_remain_visible() {
        let nodes = vec![node("host-a", 0, 8), node("host-b", 1, 16)];
        let summary = RegistryHealth::from_nodes(&nodes);

        assert_eq!(summary.expected_ranks, None);
        assert_eq!(summary.rank_coverage(), "2");
        assert_eq!(summary.world_size_label(), "8, 16");
    }

    #[test]
    fn heartbeat_age_is_a_direct_time_delta() {
        assert_eq!(format_heartbeat_age(10_000_000, 8_500_000), "1s ago");
        assert_eq!(format_heartbeat_age(120_000_000, 0), "—");
    }

    #[test]
    fn registry_filter_matches_rank_host_gpu_endpoint_and_role() {
        let node = Node {
            host: "megatron-node-07".to_string(),
            addr: "127.0.0.1:53107".to_string(),
            rank: Some(57),
            local_rank: Some(1),
            role: Some("dp=7,pp=0,tp=1".to_string()),
            role_name: Some("trainer".to_string()),
            ..Default::default()
        };

        for query in ["57", "node-07", "gpu 1", "53107", "tp=1", "trainer"] {
            assert!(node_matches_query(&node, query));
        }
        assert!(!node_matches_query(&node, "rank 2"));
    }

    #[test]
    fn pinned_process_filter_requires_all_reported_coordinates() {
        let node = Node {
            host: "megatron-node-07".to_string(),
            rank: Some(58),
            local_rank: Some(2),
            ..Default::default()
        };
        let context = InvestigationContext {
            host: Some("megatron-node-07".to_string()),
            rank: Some(58),
            device_id: Some(2),
            ..Default::default()
        };

        assert!(node_matches_context(&node, &context));
        assert!(!node_matches_context(
            &node,
            &InvestigationContext {
                device_id: Some(3),
                ..context
            }
        ));
    }
}
