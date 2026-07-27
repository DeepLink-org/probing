use dioxus::prelude::*;
use probing_proto::prelude::Node;

use crate::api::{ApiClient, StepMatrixResponse};
use crate::utils::error::AppError;

use super::super::components::{
    ClassicLink, FindingCard, FindingTone, LoadingPanel, MetricCard, NextPageHeader, SectionCard,
    UnavailablePanel,
};
use super::super::model::{format_duration, StepHealth};
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
    let health = step_state
        .as_ref()
        .and_then(|result| result.as_ref().ok())
        .map(StepHealth::from_matrix);

    rsx! {
        div { class: "space-y-5",
            NextPageHeader {
                title: "Distributed diagnosis".to_string(),
                subtitle: "Cluster completeness, rank alignment, and culprit/victim evidence before opening raw stacks or timelines.".to_string(),
                actions: rsx! {
                    ClassicLink { path: "/stacks/distributed".to_string(), label: "Open distributed flamegraph".to_string() }
                }
            }

            CompletenessFinding { node_state: node_state.clone(), step_state: step_state.clone(), health: health.clone() }

            div { class: "grid gap-3 sm:grid-cols-2 lg:grid-cols-4",
                MetricCard {
                    label: "Registered nodes".to_string(),
                    value: node_state.as_ref().and_then(|result| result.as_ref().ok()).map(|nodes| nodes.len().to_string()).unwrap_or_else(|| "—".to_string()),
                    detail: Some("cluster heartbeat registry".to_string()),
                    icon: &icondata::AiClusterOutlined,
                }
                MetricCard {
                    label: "Observed ranks".to_string(),
                    value: health.as_ref().map(|health| format!("{} / {}", health.observed_ranks, health.expected_ranks)).unwrap_or_else(|| "—".to_string()),
                    detail: Some("latest comparable step samples".to_string()),
                    icon: &icondata::AiApartmentOutlined,
                }
                MetricCard {
                    label: "Slowest rank".to_string(),
                    value: health.as_ref().and_then(|health| health.slowest_rank).map(|rank| format!("rank {rank}")).unwrap_or_else(|| "—".to_string()),
                    detail: health.as_ref().map(|health| format_duration(health.slowest_ms)),
                    icon: &icondata::AiWarningOutlined,
                }
                MetricCard {
                    label: "Tail / median".to_string(),
                    value: health.as_ref().and_then(StepHealth::slowest_ratio).map(|ratio| format!("{ratio:.2}×")).unwrap_or_else(|| "—".to_string()),
                    detail: Some("latest step rank spread".to_string()),
                    icon: &icondata::AiLineChartOutlined,
                }
            }

            div { class: "grid gap-4 xl:grid-cols-[minmax(0,1.4fr)_minmax(320px,0.6fr)]",
                SectionCard {
                    title: "Rank alignment".to_string(),
                    subtitle: Some("Latest observed train.step duration by rank.".to_string()),
                    match step_state.as_ref() {
                        None => rsx! { LoadingPanel { label: "Loading cross-rank timings".to_string() } },
                        Some(Err(error)) => rsx! { UnavailablePanel {
                            label: "Cross-rank timing unavailable".to_string(),
                            detail: error.display_message(),
                        }},
                        Some(Ok(matrix)) if matrix.samples.is_empty() => rsx! { UnavailablePanel {
                            label: "No distributed step samples".to_string(),
                            detail: "Wait for train.step spans or verify cluster fan-out.".to_string(),
                        }},
                        Some(Ok(_)) => rsx! { RankTable { health: health.clone().unwrap_or_default() } },
                    }
                }
                SectionCard {
                    title: "Evidence workspaces".to_string(),
                    div { class: "space-y-3",
                        EvidenceLink {
                            title: "Distributed stacks".to_string(),
                            detail: "Python and native frames merged across ranks.".to_string(),
                            path: "/stacks/distributed".to_string(),
                        }
                        EvidenceLink {
                            title: "Distributed Python".to_string(),
                            detail: "Python-only cluster flamegraph.".to_string(),
                            path: "/stacks/distributed/py".to_string(),
                        }
                        EvidenceLink {
                            title: "Span hierarchy".to_string(),
                            detail: "Cross-process Python and RL span evidence.".to_string(),
                            path: "/spans".to_string(),
                        }
                    }
                }
            }

            SectionCard {
                title: "Cluster members".to_string(),
                subtitle: Some("Live registry data; a failed registry request is not treated as an empty cluster.".to_string()),
                match node_state.as_ref() {
                    None => rsx! { LoadingPanel { label: "Loading cluster membership".to_string() } },
                    Some(Err(error)) => rsx! { UnavailablePanel {
                        label: "Cluster registry unavailable".to_string(),
                        detail: error.display_message(),
                    }},
                    Some(Ok(nodes)) if nodes.is_empty() => rsx! { UnavailablePanel {
                        label: "No nodes registered".to_string(),
                        detail: "This may be a single-process job or heartbeat has not started.".to_string(),
                    }},
                    Some(Ok(nodes)) => rsx! { NodeTable { nodes: nodes.clone() } },
                }
            }
        }
    }
}

#[component]
fn CompletenessFinding(
    node_state: Option<Result<Vec<Node>, AppError>>,
    step_state: Option<Result<StepMatrixResponse, AppError>>,
    health: Option<StepHealth>,
) -> Element {
    let (tone, title, detail) = match (&node_state, &step_state, &health) {
        (Some(Err(_)), _, _) => (
            FindingTone::Critical,
            "Cluster membership unknown".to_string(),
            "The node registry failed; distributed conclusions are blocked.".to_string(),
        ),
        (_, Some(Err(_)), _) => (
            FindingTone::Critical,
            "Rank fan-out failed".to_string(),
            "No local-only fallback is presented as a cluster result.".to_string(),
        ),
        (_, _, Some(health)) if !health.nodes_failed.is_empty() => (
            FindingTone::Warning,
            format!(
                "Partial evidence · {} node(s) failed",
                health.nodes_failed.len()
            ),
            format!(
                "{} of {} ranks are represented in the latest comparison.",
                health.observed_ranks, health.expected_ranks
            ),
        ),
        (Some(Ok(nodes)), _, Some(health)) => (
            FindingTone::Healthy,
            "Cluster evidence available".to_string(),
            format!(
                "{} registered nodes and {} observed ranks.",
                nodes.len(),
                health.observed_ranks
            ),
        ),
        _ => (
            FindingTone::Info,
            "Collecting distributed evidence".to_string(),
            "Membership and rank timing requests are still running.".to_string(),
        ),
    };
    rsx! {
        FindingCard {
            eyebrow: "Data completeness".to_string(),
            title,
            detail,
            tone,
        }
    }
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
                        th { class: "pb-2 pl-4 font-medium", "Relative to median" }
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

#[component]
fn EvidenceLink(title: String, detail: String, path: String) -> Element {
    rsx! {
        div { class: "rounded-lg border border-gray-200 bg-gray-50 p-3",
            div { class: "text-sm font-medium text-gray-900", "{title}" }
            p { class: "mt-1 text-xs text-gray-500", "{detail}" }
            div { class: "mt-3",
                ClassicLink { path, label: "Open proven view".to_string() }
            }
        }
    }
}

#[component]
fn NodeTable(nodes: Vec<Node>) -> Element {
    rsx! {
        div { class: "overflow-x-auto",
            table { class: "w-full min-w-[640px] text-left text-xs",
                thead { class: "text-gray-500",
                    tr {
                        th { class: "pb-2 font-medium", "Host" }
                        th { class: "pb-2 font-medium", "Address" }
                        th { class: "pb-2 font-medium", "Rank" }
                        th { class: "pb-2 font-medium", "Role" }
                        th { class: "pb-2 font-medium", "Status" }
                    }
                }
                tbody { class: "divide-y divide-gray-100",
                    for node in nodes.iter().take(64) {
                        tr {
                            td { class: "py-2 font-medium text-gray-800", "{node.host}" }
                            td { class: "py-2 font-mono text-gray-600", "{node.addr}" }
                            td { class: "py-2 text-gray-700", "{node.rank.map(|rank| rank.to_string()).unwrap_or_else(|| \"—\".to_string())}" }
                            td { class: "py-2 text-gray-700", "{node.role.clone().or_else(|| node.role_name.clone()).unwrap_or_else(|| \"—\".to_string())}" }
                            td { class: "py-2 text-gray-700", "{node.status.clone().unwrap_or_else(|| \"unknown\".to_string())}" }
                        }
                    }
                }
            }
        }
    }
}
