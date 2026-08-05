use std::collections::{BTreeMap, BTreeSet};

use dioxus::prelude::*;
use dioxus_router::Link;
use probing_proto::prelude::Node;

use crate::api::{ApiClient, StepMatrixResponse};

use super::super::components::{
    EvidenceMetric, InlineNotice, LoadingPanel, MetricCard, NoticeTone, SectionCard,
    UnavailablePanel, WorkspacePage,
};
use super::super::model::{format_duration, RegistryHealth, StepHealth};
use super::super::routes::NextRoute;
use super::super::settings::{
    DISTRIBUTED_CLUSTER_SCOPE, DISTRIBUTED_REFRESH, DISTRIBUTED_STEP_LIMIT,
};

#[component]
pub fn DistributedPage() -> Element {
    let nodes = use_resource(|| {
        let _ = *DISTRIBUTED_REFRESH.read();
        async move { ApiClient::new().get_nodes().await }
    });
    let steps = use_resource(|| {
        let _ = *DISTRIBUTED_REFRESH.read();
        let limit = *DISTRIBUTED_STEP_LIMIT.read();
        let cluster = *DISTRIBUTED_CLUSTER_SCOPE.read();
        async move { ApiClient::new().fetch_step_matrix(limit, cluster).await }
    });
    let node_state = nodes.read().clone();
    let step_state = steps.read().clone();
    let registry_health = node_state
        .as_ref()
        .and_then(|result| result.as_ref().ok())
        .map(|nodes| RegistryHealth::from_nodes(nodes));
    let health = step_state
        .as_ref()
        .and_then(|result| result.as_ref().ok())
        .map(StepHealth::from_matrix)
        .map(|health| reconcile_step_coverage(health, registry_health.as_ref()));
    let rank_comparison_subtitle = health
        .as_ref()
        .map(|health| {
            format!(
                "Latest duration for returned ranks; coverage is {} / {} registered ranks.",
                health.observed_ranks, health.expected_ranks
            )
        })
        .unwrap_or_else(|| {
            "Latest duration for returned ranks; coverage is reported independently.".to_string()
        });

    rsx! {
        WorkspacePage {
            title: "Cluster Overview".to_string(),
            subtitle: "Heartbeat coverage and latest comparable step duration by rank.".to_string(),
            actions: rsx! {
                    Link {
                        to: NextRoute::Cluster {},
                        class: "inline-flex items-center rounded-lg border border-gray-300 bg-white px-3 py-2 text-xs font-medium text-gray-700 shadow-sm hover:bg-gray-50",
                        "View nodes"
                    }
                },

            ClusterStatus {
                node_state: node_state.clone(),
                step_state: step_state.clone(),
                health: health.clone(),
                registry_health: registry_health.clone(),
            }

            div { class: "grid gap-3 md:grid-cols-2 xl:grid-cols-4",
                MetricCard {
                    label: "Hosts reporting".to_string(),
                    value: registry_health.as_ref().map(|health| health.hosts.to_string()).unwrap_or_else(|| "—".to_string()),
                    detail: Some("unique heartbeat host names".to_string()),
                    icon: &icondata::AiClusterOutlined,
                }
                MetricCard {
                    label: "Processes reporting".to_string(),
                    value: registry_health.as_ref().map(|health| health.processes.to_string()).unwrap_or_else(|| "—".to_string()),
                    detail: Some("heartbeat registry rows".to_string()),
                    icon: &icondata::AiDeploymentUnitOutlined,
                }
                MetricCard {
                    label: "Ranks with step samples".to_string(),
                    value: health.as_ref().map(|health| format!("{} / {}", health.observed_ranks, health.expected_ranks)).unwrap_or_else(|| "—".to_string()),
                    detail: Some("returned / registered world size".to_string()),
                    icon: &icondata::AiApartmentOutlined,
                }
                MetricCard {
                    label: "Observed step skew".to_string(),
                    value: health.as_ref().and_then(StepHealth::slowest_ratio).map(|ratio| format!("{ratio:.2}×")).unwrap_or_else(|| "—".to_string()),
                    detail: health.as_ref().and_then(|health| health.slowest_rank).map(|rank| format!("rank {rank} vs observed-rank median")).or_else(|| Some("slowest returned rank vs observed median".to_string())),
                    icon: &icondata::AiLineChartOutlined,
                }
            }

            SectionCard {
                title: "Host and rank placement".to_string(),
                subtitle: Some("Heartbeat rows grouped by reported host; ranks, local GPU indexes, and states are shown without inferring topology.".to_string()),
                body_class: "p-0".to_string(),
                match node_state.as_ref() {
                    None => rsx! { div { class: "p-4", LoadingPanel { label: "Grouping heartbeat rows".to_string() } } },
                    Some(Err(error)) => rsx! { div { class: "p-4", UnavailablePanel { label: "Host placement unavailable".to_string(), detail: error.display_message() } } },
                    Some(Ok(nodes)) if nodes.is_empty() => rsx! { div { class: "p-4", UnavailablePanel { label: "No heartbeat rows".to_string(), detail: "The registry request completed successfully with zero rows.".to_string() } } },
                    Some(Ok(nodes)) => rsx! { HostPlacementTable { hosts: host_placements(nodes) } },
                }
            }

            SectionCard {
                title: "Rank comparison".to_string(),
                subtitle: Some(rank_comparison_subtitle),
                match step_state.as_ref() {
                    None => rsx! { LoadingPanel { label: "Comparing rank timings".to_string() } },
                    Some(Err(_)) => rsx! { UnavailablePanel {
                        label: "Rank comparison unavailable".to_string(),
                        detail: "Cluster fan-out could not collect comparable steps. Check Nodes, then refresh the evidence.".to_string(),
                    }},
                    Some(Ok(matrix)) if matrix.samples.is_empty() => rsx! { UnavailablePanel {
                        label: "No comparable rank samples yet".to_string(),
                        detail: "Wait for train.step sampling to begin, then refresh the cluster evidence.".to_string(),
                    }},
                    Some(Ok(_)) => rsx! { RankTable { health: health.clone().unwrap_or_default() } },
                }
            }

        }
    }
}

#[component]
pub fn DistributedStatusPage() -> Element {
    let runtime_debug = use_resource(|| {
        let _ = *DISTRIBUTED_REFRESH.read();
        async move { ApiClient::new().get_pytorch_runtime_debug().await }
    });
    let state = runtime_debug.read().clone();

    rsx! {
        WorkspacePage {
            title: "Distributed Status".to_string(),
            subtitle: "Inspect current PyTorch wait scopes and rendezvous state. Each capability reports availability independently.".to_string(),
            match state {
                None => rsx! { LoadingPanel { label: "Reading distributed runtime state".to_string() } },
                Some(Err(error)) => rsx! { UnavailablePanel {
                    label: "Distributed status unavailable".to_string(),
                    detail: error.display_message(),
                }},
                Some(Ok(snapshot)) => rsx! {
                    div { class: "grid items-start gap-4 xl:grid-cols-2",
                        WaitCounterPanel { snapshot: snapshot.wait_counters }
                        TcpStorePanel { snapshot: snapshot.tcpstore }
                    }
                },
            }
        }
    }
}

#[component]
fn WaitCounterPanel(snapshot: crate::api::WaitCounterSnapshot) -> Element {
    if !snapshot.available {
        return rsx! { UnavailablePanel {
            label: "Wait counters unavailable".to_string(),
            detail: snapshot.error.unwrap_or_else(|| "This PyTorch build does not expose the wait-counter worker handler.".to_string()),
        }};
    }
    let active = snapshot
        .counters
        .iter()
        .filter(|counter| counter.active_count > 0)
        .count();
    let calls: i64 = snapshot
        .counters
        .iter()
        .map(|counter| counter.total_calls)
        .sum();
    let maximum = snapshot
        .counters
        .iter()
        .map(|counter| counter.max_time_us)
        .max()
        .unwrap_or_default();
    let mut counters = snapshot.counters;
    counters.sort_by_key(|counter| (counter.active_count, counter.max_time_us));
    counters.reverse();

    rsx! {
        div { class: "min-w-0 rounded-lg border border-gray-200 p-3",
            div { class: "flex items-baseline justify-between",
                h3 { class: "text-sm font-semibold text-gray-900", "Wait states · rank {snapshot.rank}" }
                span { class: "text-xs text-gray-500", "{snapshot.source} · {counters.len()} counters" }
            }
            div { class: "mt-3 grid grid-cols-3 divide-x divide-gray-200 text-center",
                RuntimeMetric { label: "Active", value: active.to_string() }
                RuntimeMetric { label: "Calls", value: calls.to_string() }
                RuntimeMetric { label: "Max", value: format_wait_us(maximum) }
            }
            div { class: "mt-3 divide-y divide-gray-100 border-t border-gray-100",
                for counter in counters.iter().take(6) {
                    div { class: "grid grid-cols-[minmax(0,1fr)_auto_auto] items-center gap-3 py-2 text-xs",
                        div { class: "break-all font-mono text-xs text-gray-700", "{counter.name}" }
                        span { class: if counter.active_count > 0 { "font-medium text-amber-700" } else { "text-gray-500" }, "active {counter.active_count}" }
                        span { class: "font-mono text-gray-600", "{format_wait_us(counter.max_time_us)}" }
                    }
                }
                if counters.is_empty() {
                    p { class: "py-3 text-xs text-gray-500", "No instrumented wait counters observed." }
                }
            }
        }
    }
}

#[component]
fn TcpStorePanel(snapshot: crate::api::TcpStoreSnapshot) -> Element {
    if !snapshot.available {
        return rsx! { UnavailablePanel {
            label: "TCPStore unavailable".to_string(),
            detail: snapshot.error.unwrap_or_else(|| "No torchrun rendezvous store is available in this process.".to_string()),
        }};
    }
    let visibility = if snapshot.catalog_available {
        "Complete"
    } else {
        "Known keys"
    };

    rsx! {
        div { class: "min-w-0 rounded-lg border border-gray-200 p-3",
            div { class: "flex items-baseline justify-between",
                h3 { class: "text-sm font-semibold text-gray-900", "Rendezvous store" }
                span { class: "text-xs text-gray-500", "read only" }
            }
            div { class: "mt-3 grid grid-cols-3 divide-x divide-gray-200 text-center",
                RuntimeMetric { label: "Store keys", value: snapshot.total_keys.to_string() }
                RuntimeMetric { label: "Identified", value: snapshot.identified_keys.to_string() }
                RuntimeMetric { label: "Catalog", value: visibility.to_string() }
            }
            if !snapshot.facts.is_empty() {
                div { class: "mt-3 grid grid-cols-2 gap-x-4 gap-y-2 border-t border-gray-100 pt-3",
                    for fact in snapshot.facts.iter() {
                        div { class: "min-w-0",
                            div { class: "text-xs uppercase tracking-wide text-gray-500", "{fact.label}" }
                            div { class: "break-all font-mono text-xs text-gray-700", "{fact.value}" }
                        }
                    }
                }
            }
            if !snapshot.entries.is_empty() {
                div { class: "mt-3 divide-y divide-gray-100 border-t border-gray-100",
                    for entry in snapshot.entries.iter().take(6) {
                        div { class: "grid grid-cols-[auto_minmax(0,1fr)_auto] items-center gap-3 py-2 text-xs",
                            span { class: "rounded bg-gray-100 px-1.5 py-0.5 text-xs text-gray-600", "{entry.category}" }
                            div { class: "min-w-0",
                                div { class: "break-all font-mono text-xs text-gray-700", "{entry.key}" }
                                if !entry.value_preview.is_empty() {
                                    div { class: "break-all text-xs text-gray-500", "{entry.value_preview}" }
                                }
                            }
                            span { class: "font-mono text-xs text-gray-500", "{entry.value_size} B" }
                        }
                    }
                }
            }
            if !snapshot.catalog_available {
                p { class: "mt-3 border-t border-gray-100 pt-3 text-xs leading-relaxed text-gray-500",
                    "This PyTorch build cannot enumerate arbitrary keys. Identified entries are non-blocking probes of known torchrun and Probing key names; {snapshot.total_keys.saturating_sub(snapshot.identified_keys)} keys remain unnamed."
                }
            } else if snapshot.entries.is_empty() {
                p { class: "mt-3 border-t border-gray-100 pt-3 text-xs text-gray-500", "The rendezvous store contains no keys." }
            }
        }
    }
}

#[component]
fn RuntimeMetric(label: &'static str, value: String) -> Element {
    rsx! { EvidenceMetric { label, value } }
}

fn format_wait_us(value: i64) -> String {
    if value >= 1_000_000 {
        format!("{:.2}s", value as f64 / 1_000_000.0)
    } else if value >= 1_000 {
        format!("{:.1}ms", value as f64 / 1_000.0)
    } else {
        format!("{value}µs")
    }
}

#[component]
fn ClusterStatus(
    node_state: Option<Result<Vec<probing_proto::prelude::Node>, crate::utils::error::AppError>>,
    step_state: Option<Result<StepMatrixResponse, crate::utils::error::AppError>>,
    health: Option<StepHealth>,
    registry_health: Option<RegistryHealth>,
) -> Element {
    let node_request = request_state(&node_state);
    let step_request = request_state(&step_state);
    let assessment = cluster_assessment(
        node_request,
        registry_health.as_ref(),
        step_request,
        health.as_ref(),
    );
    rsx! {
        InlineNotice {
            title: assessment.title,
            detail: assessment.detail,
            tone: assessment.tone,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RequestState {
    Loading,
    Ready,
    Failed,
}

#[derive(Debug, PartialEq, Eq)]
struct ClusterAssessment {
    tone: NoticeTone,
    title: String,
    detail: String,
}

fn request_state<T, E>(state: &Option<Result<T, E>>) -> RequestState {
    match state {
        None => RequestState::Loading,
        Some(Ok(_)) => RequestState::Ready,
        Some(Err(_)) => RequestState::Failed,
    }
}

fn reconcile_step_coverage(
    mut health: StepHealth,
    registry: Option<&RegistryHealth>,
) -> StepHealth {
    if let Some(expected) = registry.and_then(|registry| registry.expected_ranks) {
        health.expected_ranks = expected.max(health.observed_ranks);
    }
    health
}

fn cluster_assessment(
    node_request: RequestState,
    registry: Option<&RegistryHealth>,
    step_request: RequestState,
    health: Option<&StepHealth>,
) -> ClusterAssessment {
    let assessment = |tone, title: &str, detail: String| ClusterAssessment {
        tone,
        title: title.to_string(),
        detail,
    };

    if node_request == RequestState::Failed {
        return assessment(
            NoticeTone::Info,
            "Heartbeat registry request failed",
            "No heartbeat coverage value is available for this refresh.".to_string(),
        );
    }
    if node_request == RequestState::Ready && registry.is_some_and(|health| health.processes == 0) {
        return assessment(
            NoticeTone::Warning,
            "0 processes reported",
            "The heartbeat registry returned zero rows.".to_string(),
        );
    }
    if step_request == RequestState::Failed {
        return assessment(
            NoticeTone::Info,
            "Rank comparison request failed",
            "Heartbeat registry data and comparable train.step data have independent availability."
                .to_string(),
        );
    }

    if let Some(health) = health {
        if !health.nodes_failed.is_empty() {
            return assessment(
                NoticeTone::Warning,
                &format!("{} peer endpoint(s) failed", health.nodes_failed.len()),
                format!(
                    "{} of {} registered ranks have a returned step sample; endpoint failures and missing ranks are different counts.",
                    health.observed_ranks, health.expected_ranks
                ),
            );
        }
        if health.observed_ranks == 0 {
            return assessment(
                NoticeTone::Info,
                "0 comparable ranks",
                "No rank returned a comparable train.step sample.".to_string(),
            );
        }
        if health.observed_ranks < health.expected_ranks {
            return assessment(
                NoticeTone::Warning,
                &format!(
                    "{} / {} ranks represented",
                    health.observed_ranks, health.expected_ranks
                ),
                format!(
                    "{} of {} ranks are present in the latest step comparison.",
                    health.observed_ranks, health.expected_ranks
                ),
            );
        }
        if let Some(ratio) = health.slowest_ratio().filter(|ratio| *ratio > 1.2) {
            return assessment(
                NoticeTone::Warning,
                &format!("Observed slowest / median: {ratio:.2}×"),
                format!(
                    "The ratio is derived only from the {} returned ranks shown below.",
                    health.observed_ranks
                ),
            );
        }

        return assessment(
            NoticeTone::Info,
            &format!(
                "{} / {} ranks represented",
                health.observed_ranks, health.expected_ranks
            ),
            format!(
                "{} host(s), {} process(es); slowest / median is {}.",
                registry.map(|health| health.hosts).unwrap_or_default(),
                registry.map(|health| health.processes).unwrap_or_default(),
                health
                    .slowest_ratio()
                    .map(|ratio| format!("{ratio:.2}×"))
                    .unwrap_or_else(|| "—".to_string())
            ),
        );
    }

    assessment(
        NoticeTone::Info,
        "Loading cluster coverage",
        "Node heartbeats and comparable rank timings are requested independently.".to_string(),
    )
}

#[component]
fn RankTable(health: StepHealth) -> Element {
    rsx! {
        div { class: "overflow-x-auto",
            table { class: "w-full text-left text-xs",
                thead { class: "text-gray-500",
                    tr {
                        th { class: "pb-2 font-medium", "Rank" }
                        th { class: "pb-2 text-right font-medium", "Duration" }
                        th { class: "pb-2 pl-4 font-medium", "Relative to observed median" }
                    }
                }
                tbody { class: "divide-y divide-gray-100",
                    for (rank, duration) in health.rank_durations.iter().take(24) {
                        RankTableRow {
                            rank: *rank,
                            duration: *duration,
                            median: health.median_ms,
                            slowest: Some(*rank) == health.slowest_rank,
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn RankTableRow(rank: i32, duration: f64, median: Option<f64>, slowest: bool) -> Element {
    let width = median
        .filter(|median| *median > 0.0)
        .map(|median| (duration / median * 50.0).clamp(2.0, 100.0))
        .unwrap_or(2.0);
    let width_style = format!("width: {width:.1}%;");

    rsx! {
        tr {
            td { class: "py-2 font-medium text-gray-800", "rank {rank}" }
            td { class: "py-2 text-right font-mono text-gray-700", "{format_duration(Some(duration))}" }
            td { class: "py-2 pl-4",
                div { class: "h-2 overflow-hidden rounded-full bg-gray-100",
                    div {
                        class: if slowest { "h-full rounded-full bg-amber-500" } else { "h-full rounded-full bg-blue-500" },
                        style: "{width_style}",
                    }
                }
            }
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct HostPlacement {
    host: String,
    processes: usize,
    ranks: Vec<i32>,
    local_devices: Vec<i32>,
    states: Vec<(String, usize)>,
}

fn host_placements(nodes: &[Node]) -> Vec<HostPlacement> {
    let mut grouped = BTreeMap::<String, Vec<&Node>>::new();
    for node in nodes {
        let host = if node.host.trim().is_empty() {
            "(host not reported)".to_string()
        } else {
            node.host.clone()
        };
        grouped.entry(host).or_default().push(node);
    }
    grouped
        .into_iter()
        .map(|(host, nodes)| {
            let ranks = nodes
                .iter()
                .filter_map(|node| node.rank)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            let local_devices = nodes
                .iter()
                .filter_map(|node| node.local_rank)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            let mut states = BTreeMap::<String, usize>::new();
            for node in &nodes {
                let state = node
                    .status
                    .as_deref()
                    .filter(|state| !state.trim().is_empty())
                    .unwrap_or("not reported")
                    .to_string();
                *states.entry(state).or_default() += 1;
            }
            HostPlacement {
                host,
                processes: nodes.len(),
                ranks,
                local_devices,
                states: states.into_iter().collect(),
            }
        })
        .collect()
}

#[component]
fn HostPlacementTable(hosts: Vec<HostPlacement>) -> Element {
    rsx! { div { class: "overflow-x-auto", table { class: "w-full text-left text-xs",
        thead { class: "bg-gray-50 uppercase tracking-wide text-gray-500", tr {
            th { class: "px-4 py-2", "Host" } th { class: "px-4 py-2 text-right", "Processes" }
            th { class: "px-4 py-2", "Ranks" } th { class: "px-4 py-2", "Local GPU indexes" }
            th { class: "px-4 py-2", "Reported states" }
        } }
        tbody { class: "divide-y divide-gray-100", for host in hosts { tr {
            { let ranks = format_indexes(&host.ranks, "R"); let devices = format_indexes(&host.local_devices, "GPU "); rsx! {
            td { class: "px-4 py-2 font-mono font-medium text-gray-800", "{host.host}" }
            td { class: "px-4 py-2 text-right font-mono", "{host.processes}" }
            td { class: "px-4 py-2 font-mono text-gray-700", "{ranks}" }
            td { class: "px-4 py-2 font-mono text-gray-700", "{devices}" }
            td { class: "px-4 py-2 text-gray-600", "{format_states(&host.states)}" }
            } }
        } } }
    } } }
}

fn format_indexes(values: &[i32], prefix: &str) -> String {
    if values.is_empty() {
        return "—".to_string();
    }
    values
        .iter()
        .map(|value| format!("{prefix}{value}"))
        .collect::<Vec<_>>()
        .join("  ")
}

fn format_states(states: &[(String, usize)]) -> String {
    states
        .iter()
        .map(|(state, count)| format!("{state} ×{count}"))
        .collect::<Vec<_>>()
        .join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assessment_explains_missing_ranks() {
        let registry = RegistryHealth {
            hosts: 2,
            processes: 8,
            ..Default::default()
        };
        let health = StepHealth {
            observed_ranks: 6,
            expected_ranks: 8,
            ..Default::default()
        };
        let result = cluster_assessment(
            RequestState::Ready,
            Some(&registry),
            RequestState::Ready,
            Some(&health),
        );

        assert_eq!(result.tone, NoticeTone::Warning);
        assert_eq!(result.title, "6 / 8 ranks represented");
    }

    #[test]
    fn assessment_explains_rank_skew() {
        let registry = RegistryHealth {
            hosts: 2,
            processes: 8,
            ..Default::default()
        };
        let health = StepHealth {
            median_ms: Some(10.0),
            slowest_ms: Some(14.0),
            observed_ranks: 8,
            expected_ranks: 8,
            ..Default::default()
        };
        let result = cluster_assessment(
            RequestState::Ready,
            Some(&registry),
            RequestState::Ready,
            Some(&health),
        );

        assert_eq!(result.tone, NoticeTone::Warning);
        assert_eq!(result.title, "Observed slowest / median: 1.40×");
    }

    #[test]
    fn registry_world_size_overrides_partial_step_matrix_rank_count() {
        let registry = RegistryHealth {
            hosts: 8,
            processes: 64,
            observed_ranks: 64,
            expected_ranks: Some(64),
            ..Default::default()
        };
        let health = StepHealth {
            observed_ranks: 8,
            expected_ranks: 8,
            nodes_failed: vec!["peer-a".to_string()],
            ..Default::default()
        };

        let reconciled = reconcile_step_coverage(health, Some(&registry));
        assert_eq!(reconciled.observed_ranks, 8);
        assert_eq!(reconciled.expected_ranks, 64);

        let assessment = cluster_assessment(
            RequestState::Ready,
            Some(&registry),
            RequestState::Ready,
            Some(&reconciled),
        );
        assert_eq!(assessment.title, "1 peer endpoint(s) failed");
        assert!(assessment.detail.contains("8 of 64 registered ranks"));
    }

    #[test]
    fn assessment_reports_complete_rank_coverage() {
        let registry = RegistryHealth {
            hosts: 2,
            processes: 8,
            ..Default::default()
        };
        let health = StepHealth {
            median_ms: Some(10.0),
            slowest_ms: Some(11.0),
            observed_ranks: 8,
            expected_ranks: 8,
            ..Default::default()
        };
        let result = cluster_assessment(
            RequestState::Ready,
            Some(&registry),
            RequestState::Ready,
            Some(&health),
        );

        assert_eq!(result.tone, NoticeTone::Info);
        assert_eq!(result.title, "8 / 8 ranks represented");
        assert_eq!(
            result.detail,
            "2 host(s), 8 process(es); slowest / median is 1.10×."
        );
    }

    #[test]
    fn wait_duration_uses_readable_units() {
        assert_eq!(format_wait_us(42), "42µs");
        assert_eq!(format_wait_us(1_500), "1.5ms");
        assert_eq!(format_wait_us(2_000_000), "2.00s");
    }

    #[test]
    fn host_placement_keeps_process_rank_device_and_state_counts_separate() {
        let nodes = vec![
            Node {
                host: "node-a".into(),
                rank: Some(0),
                local_rank: Some(0),
                status: Some("running".into()),
                ..Default::default()
            },
            Node {
                host: "node-a".into(),
                rank: Some(1),
                local_rank: Some(1),
                status: Some("running".into()),
                ..Default::default()
            },
            Node {
                host: "node-b".into(),
                rank: Some(2),
                local_rank: Some(0),
                status: None,
                ..Default::default()
            },
        ];

        let hosts = host_placements(&nodes);
        assert_eq!(hosts.len(), 2);
        assert_eq!(hosts[0].processes, 2);
        assert_eq!(hosts[0].ranks, vec![0, 1]);
        assert_eq!(hosts[0].local_devices, vec![0, 1]);
        assert_eq!(hosts[0].states, vec![("running".to_string(), 2)]);
        assert_eq!(hosts[1].states, vec![("not reported".to_string(), 1)]);
    }
}
