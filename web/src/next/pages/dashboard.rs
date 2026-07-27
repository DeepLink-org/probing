use dioxus::prelude::*;
use dioxus_router::Link;

use crate::api::{ApiClient, CpuSnapshot, GpuSnapshot, StepMatrixResponse};
use crate::hooks::{use_page_visible, use_poll_tick_gated};
use crate::overhead::OverheadSnapshot;
use crate::utils::error::AppError;

use super::super::components::{
    FindingCard, FindingTone, LoadingPanel, MetricCard, NextPageHeader, SectionCard,
    UnavailablePanel,
};
use super::super::model::{format_duration, format_percent, GpuHealth, StepHealth};
use super::super::routes::NextRoute;
use super::super::settings::{DASHBOARD_AUTO_REFRESH, DASHBOARD_MANUAL_REFRESH};

const POLL_MS: u32 = 5_000;

#[component]
pub fn DashboardPage() -> Element {
    let visible = use_page_visible();
    let poll = use_poll_tick_gated(POLL_MS, Some(visible));
    let refresh_key = use_memo(move || {
        if *DASHBOARD_AUTO_REFRESH.read() {
            poll()
        } else {
            *DASHBOARD_MANUAL_REFRESH.read()
        }
    });
    let refresh_tick = refresh_key();

    let steps = use_resource(move || {
        let _ = refresh_key();
        async move { ApiClient::new().fetch_step_matrix(256, true).await }
    });
    let gpu = use_resource(move || {
        let _ = refresh_key();
        async move { ApiClient::new().fetch_gpu_latest().await }
    });
    let cpu = use_resource(move || {
        let _ = refresh_key();
        async move { ApiClient::new().fetch_cpu_latest().await }
    });
    let nodes = use_resource(move || {
        let _ = refresh_key();
        async move { ApiClient::new().get_nodes().await }
    });
    let overhead = use_resource(move || {
        let _ = refresh_key();
        async move { ApiClient::new().fetch_overhead_summary().await }
    });

    let step_state = steps.read().clone();
    let gpu_state = gpu.read().clone();
    let cpu_state = cpu.read().clone();
    let node_state = nodes.read().clone();
    let overhead_state = overhead.read().clone();
    let step_health = step_state
        .as_ref()
        .and_then(|result| result.as_ref().ok())
        .map(StepHealth::from_matrix);
    let gpu_health = gpu_state
        .as_ref()
        .and_then(|result| result.as_ref().ok())
        .map(|snapshots| GpuHealth::from_snapshots(snapshots));
    let cpu_snapshot = cpu_state
        .as_ref()
        .and_then(|result| result.as_ref().ok())
        .and_then(Clone::clone);
    let node_count = node_state
        .as_ref()
        .and_then(|result| result.as_ref().ok())
        .map(Vec::len);
    let overhead_pct = overhead_state
        .as_ref()
        .and_then(|result| result.as_ref().ok())
        .and_then(|frame| OverheadSnapshot::from_summary(frame).dispatch_overhead_pct);
    let poll_label = if *DASHBOARD_AUTO_REFRESH.read() {
        format!(
            "Live · refreshes every {}s · tick {refresh_tick}",
            POLL_MS / 1000
        )
    } else {
        format!("Manual refresh · tick {refresh_tick}")
    };

    rsx! {
        div { class: "space-y-5",
            NextPageHeader {
                title: "Training job health".to_string(),
                subtitle: "The first screen for algorithm engineers: progress, tail latency, rank health, accelerators, and the next diagnostic action.".to_string(),
                actions: rsx! {
                    span { class: "text-xs text-gray-500", "{poll_label}" }
                    Link {
                        to: NextRoute::Investigate {},
                        class: "inline-flex items-center rounded-lg bg-blue-600 px-3 py-2 text-xs font-medium text-white hover:bg-blue-700",
                        "Investigate"
                    }
                }
            }

            FindingsRow {
                step_state: step_state.clone(),
                step_health: step_health.clone(),
                gpu_state: gpu_state.clone(),
                gpu_health: gpu_health.clone(),
            }

            div { class: "grid gap-3 sm:grid-cols-2 xl:grid-cols-5",
                MetricCard {
                    label: "Training progress".to_string(),
                    value: step_health.as_ref().and_then(|health| health.latest_step).map(|step| step.to_string()).unwrap_or_else(|| "—".to_string()),
                    detail: Some("latest coordinated step".to_string()),
                    icon: &icondata::AiFieldTimeOutlined,
                }
                MetricCard {
                    label: "Step median / P95".to_string(),
                    value: step_health.as_ref().map(|health| format!("{} / {}", format_duration(health.median_ms), format_duration(health.p95_ms))).unwrap_or_else(|| "—".to_string()),
                    detail: Some("latest sample per observed rank".to_string()),
                    icon: &icondata::AiLineChartOutlined,
                }
                MetricCard {
                    label: "Rank coverage".to_string(),
                    value: step_health.as_ref().map(|health| format!("{} / {}", health.observed_ranks, health.expected_ranks)).unwrap_or_else(|| "—".to_string()),
                    detail: step_health.as_ref().and_then(|health| health.completeness_pct()).map(|value| format!("{value:.1}% complete")),
                    icon: &icondata::AiClusterOutlined,
                }
                MetricCard {
                    label: "GPU utilization".to_string(),
                    value: gpu_health.as_ref().map(|health| format_percent(health.average_util_pct)).unwrap_or_else(|| "—".to_string()),
                    detail: gpu_health.as_ref().map(|health| format!("{} device(s) · memory {}", health.device_count, format_percent(health.average_memory_pct))),
                    icon: &icondata::AiDashboardOutlined,
                }
                MetricCard {
                    label: "Probe overhead".to_string(),
                    value: format_percent(overhead_pct),
                    detail: Some(match (node_count, cpu_snapshot.as_ref()) {
                        (Some(nodes), Some(cpu)) => format!("{nodes} nodes · CPU {:.1}%", cpu.cpu_total_pct),
                        (Some(nodes), None) => format!("{nodes} nodes"),
                        (None, Some(cpu)) => format!("CPU {:.1}%", cpu.cpu_total_pct),
                        _ => "collecting runtime health".to_string(),
                    }),
                    icon: &icondata::CgPerformance,
                }
            }

            div { class: "grid gap-4 xl:grid-cols-[minmax(0,2fr)_minmax(300px,1fr)]",
                SectionCard {
                    title: "Rank latency distribution".to_string(),
                    subtitle: Some("Latest observed step per rank; slowest ranks are shown first.".to_string()),
                    match step_state.as_ref() {
                        None => rsx! { LoadingPanel { label: "Loading training steps".to_string() } },
                        Some(Err(error)) => rsx! { UnavailablePanel {
                            label: "Training step data unavailable".to_string(),
                            detail: error.display_message(),
                        }},
                        Some(Ok(matrix)) if matrix.samples.is_empty() => rsx! { UnavailablePanel {
                            label: "No train.step spans yet".to_string(),
                            detail: "Enable TorchProbe step tracing or wait for the first completed step.".to_string(),
                        }},
                        Some(Ok(_)) => rsx! { RankLatencyBars { health: step_health.clone().unwrap_or_default() } },
                    }
                }
                SectionCard {
                    title: "Recommended next action".to_string(),
                    subtitle: Some("Evidence-backed shortcuts, not a generic status list.".to_string()),
                    RecommendationPanel { health: step_health.clone(), gpu: gpu_health.clone() }
                }
            }

            SectionCard {
                title: "Resource and process details".to_string(),
                subtitle: Some("Existing CPU, GPU, thread, and process views remain available as secondary evidence.".to_string()),
                body_class: "p-4 grid gap-3 sm:grid-cols-2 lg:grid-cols-4".to_string(),
                {resource_summary(cpu_state, gpu_state, node_state, overhead_state)}
            }
        }
    }
}

#[component]
fn FindingsRow(
    step_state: Option<Result<StepMatrixResponse, AppError>>,
    step_health: Option<StepHealth>,
    gpu_state: Option<Result<Vec<GpuSnapshot>, AppError>>,
    gpu_health: Option<GpuHealth>,
) -> Element {
    let step_finding = match (&step_state, &step_health) {
        (None, _) => (
            FindingTone::Info,
            "Collecting step evidence".to_string(),
            "The dashboard will rank findings after the first cluster sample.".to_string(),
        ),
        (Some(Err(_)), _) => (
            FindingTone::Critical,
            "Step diagnosis unavailable".to_string(),
            "The failure is surfaced explicitly; no healthy conclusion is inferred.".to_string(),
        ),
        (Some(Ok(matrix)), _) if matrix.samples.is_empty() => (
            FindingTone::Info,
            "Waiting for train.step spans".to_string(),
            "No step latency sample has been observed yet.".to_string(),
        ),
        (_, Some(health)) if !health.nodes_failed.is_empty() => (
            FindingTone::Critical,
            format!(
                "Partial cluster result · {} failed",
                health.nodes_failed.len()
            ),
            "Rank comparisons remain visible, but conclusions are marked incomplete.".to_string(),
        ),
        (_, Some(health)) if health.slowest_ratio().is_some_and(|ratio| ratio >= 1.5) => (
            FindingTone::Warning,
            format!(
                "rank {} is {:.2}× slower",
                health.slowest_rank.unwrap_or_default(),
                health.slowest_ratio().unwrap_or_default()
            ),
            format!(
                "{} versus rank median {}.",
                format_duration(health.slowest_ms),
                format_duration(health.median_ms)
            ),
        ),
        (_, Some(_)) => (
            FindingTone::Healthy,
            "No severe rank outlier".to_string(),
            "Latest rank timings are within the current outlier threshold.".to_string(),
        ),
        _ => (
            FindingTone::Info,
            "Step health unknown".to_string(),
            "No usable cluster timing sample was returned.".to_string(),
        ),
    };

    let gpu_finding = match (&gpu_state, &gpu_health) {
        (None, _) => (
            FindingTone::Info,
            "Collecting accelerator evidence".to_string(),
            "GPU utilization and memory pressure are loading.".to_string(),
        ),
        (Some(Err(_)), _) => (
            FindingTone::Critical,
            "GPU metrics unavailable".to_string(),
            "Accelerator health is unknown rather than treated as idle.".to_string(),
        ),
        (Some(Ok(rows)), _) if rows.is_empty() => (
            FindingTone::Info,
            "No GPU device reported".to_string(),
            "This may be a CPU job or an unavailable GPU collector.".to_string(),
        ),
        (_, Some(health)) if health.average_util_pct.is_some_and(|value| value < 30.0) => (
            FindingTone::Warning,
            "Low GPU utilization observed".to_string(),
            format!(
                "Average utilization {} across {} device(s); inspect input and synchronization waits.",
                format_percent(health.average_util_pct),
                health.device_count
            ),
        ),
        (_, Some(health)) => (
            FindingTone::Healthy,
            "Accelerators are active".to_string(),
            format!(
                "Average GPU utilization {} across {} device(s).",
                format_percent(health.average_util_pct),
                health.device_count
            ),
        ),
        _ => (
            FindingTone::Info,
            "GPU health unknown".to_string(),
            "No usable accelerator sample was returned.".to_string(),
        ),
    };

    rsx! {
        div { class: "grid gap-3 lg:grid-cols-2",
            FindingCard {
                eyebrow: "Highest-priority finding".to_string(),
                title: step_finding.1,
                detail: step_finding.2,
                tone: step_finding.0,
                action: rsx! {
                    Link {
                        to: NextRoute::Training {},
                        class: "inline-flex rounded-lg border border-current/20 bg-white/70 px-3 py-1.5 text-xs font-medium hover:bg-white",
                        "Open training evidence"
                    }
                }
            }
            FindingCard {
                eyebrow: "Accelerator signal".to_string(),
                title: gpu_finding.1,
                detail: gpu_finding.2,
                tone: gpu_finding.0,
                action: rsx! {
                    Link {
                        to: NextRoute::Profiles {},
                        class: "inline-flex rounded-lg border border-current/20 bg-white/70 px-3 py-1.5 text-xs font-medium hover:bg-white",
                        "Open profiles"
                    }
                }
            }
        }
    }
}

#[component]
fn RankLatencyBars(health: StepHealth) -> Element {
    let maximum = health
        .rank_durations
        .first()
        .map(|(_, duration)| *duration)
        .unwrap_or(1.0)
        .max(1.0);
    rsx! {
        div { class: "space-y-3",
            for (rank, duration) in health.rank_durations.iter().take(10) {
                RankLatencyBar {
                    rank: *rank,
                    duration: *duration,
                    maximum,
                    slowest: Some(*rank) == health.slowest_rank,
                }
            }
            if health.rank_durations.is_empty() {
                UnavailablePanel {
                    label: "No comparable rank samples".to_string(),
                    detail: "The response contained no finite step durations.".to_string(),
                }
            }
        }
    }
}

#[component]
fn RankLatencyBar(rank: i32, duration: f64, maximum: f64, slowest: bool) -> Element {
    let width_style = format!(
        "width: {:.1}%;",
        (duration / maximum * 100.0).clamp(2.0, 100.0)
    );

    rsx! {
        div { class: "grid grid-cols-[4.5rem_minmax(0,1fr)_5.5rem] items-center gap-3",
            div { class: "text-xs font-medium text-gray-700", "rank {rank}" }
            div { class: "h-2.5 overflow-hidden rounded-full bg-gray-100",
                div {
                    class: if slowest { "h-full rounded-full bg-amber-500" } else { "h-full rounded-full bg-blue-500" },
                    style: "{width_style}",
                }
            }
            div { class: "text-right text-xs tabular-nums text-gray-600", "{format_duration(Some(duration))}" }
        }
    }
}

#[component]
fn RecommendationPanel(health: Option<StepHealth>, gpu: Option<GpuHealth>) -> Element {
    let slow_rank = health
        .as_ref()
        .and_then(|health| health.slowest_rank)
        .map(|rank| format!("Start with rank {rank}"));
    let action = slow_rank.unwrap_or_else(|| "Run a job health overview".to_string());
    rsx! {
        div { class: "space-y-4",
            div {
                div { class: "text-sm font-semibold text-gray-900", "1 · {action}" }
                p { class: "mt-1 text-xs leading-relaxed text-gray-500",
                    if health.as_ref().and_then(StepHealth::slowest_ratio).is_some_and(|ratio| ratio >= 1.5) {
                        "Compare its input, compute, and collective phases against the rank median."
                    } else {
                        "Collect a stable rank window before selecting a culprit."
                    }
                }
            }
            div { class: "border-t border-gray-100 pt-4",
                div { class: "text-sm font-semibold text-gray-900", "2 · Validate accelerator pressure" }
                p { class: "mt-1 text-xs leading-relaxed text-gray-500",
                    "Observed average: {gpu.as_ref().map(|value| format_percent(value.average_util_pct)).unwrap_or_else(|| \"—\".to_string())}. Correlate low utilization with input or communication waits."
                }
            }
            div { class: "flex flex-wrap gap-2 border-t border-gray-100 pt-4",
                Link {
                    to: NextRoute::Investigate {},
                    class: "rounded-lg bg-blue-600 px-3 py-2 text-xs font-medium text-white hover:bg-blue-700",
                    "Run diagnostic skill"
                }
                Link {
                    to: NextRoute::Distributed {},
                    class: "rounded-lg border border-gray-300 px-3 py-2 text-xs font-medium text-gray-700 hover:bg-gray-50",
                    "Inspect rank alignment"
                }
            }
        }
    }
}

fn resource_summary(
    cpu_state: Option<Result<Option<CpuSnapshot>, AppError>>,
    gpu_state: Option<Result<Vec<GpuSnapshot>, AppError>>,
    node_state: Option<Result<Vec<probing_proto::prelude::Node>, AppError>>,
    overhead_state: Option<Result<probing_proto::prelude::DataFrame, AppError>>,
) -> Element {
    let cpu_copy = match cpu_state {
        None => ("CPU".to_string(), "Loading…".to_string()),
        Some(Ok(Some(cpu))) => (
            format!("CPU {:.1}%", cpu.cpu_total_pct),
            format!("{} threads · RSS {} KB", cpu.thread_count, cpu.rss_kb),
        ),
        Some(Ok(None)) => ("CPU".to_string(), "Collecting samples".to_string()),
        Some(Err(_)) => (
            "CPU unavailable".to_string(),
            "Collector/query failed".to_string(),
        ),
    };
    let gpu_copy = match gpu_state {
        None => ("GPU".to_string(), "Loading…".to_string()),
        Some(Ok(rows)) => (
            format!("{} GPU device(s)", rows.len()),
            format!(
                "average {}",
                format_percent(GpuHealth::from_snapshots(&rows).average_util_pct)
            ),
        ),
        Some(Err(_)) => (
            "GPU unavailable".to_string(),
            "Collector/query failed".to_string(),
        ),
    };
    let cluster_copy = match node_state {
        None => ("Cluster".to_string(), "Loading…".to_string()),
        Some(Ok(nodes)) => (
            format!("{} node(s)", nodes.len()),
            format!(
                "{} ranked peer(s)",
                nodes.iter().filter(|node| node.rank.is_some()).count()
            ),
        ),
        Some(Err(_)) => (
            "Cluster unavailable".to_string(),
            "Node registry failed".to_string(),
        ),
    };
    let overhead_copy = match overhead_state {
        None => ("Probe overhead".to_string(), "Loading…".to_string()),
        Some(Ok(frame)) => (
            format_percent(OverheadSnapshot::from_summary(&frame).dispatch_overhead_pct),
            "dispatch path estimate".to_string(),
        ),
        Some(Err(_)) => (
            "Overhead unavailable".to_string(),
            "No health conclusion inferred".to_string(),
        ),
    };
    rsx! {
        for (title, detail) in [cpu_copy, gpu_copy, cluster_copy, overhead_copy] {
            div { class: "rounded-lg bg-gray-50 px-3 py-3",
                div { class: "text-sm font-medium text-gray-900", "{title}" }
                div { class: "mt-1 text-xs text-gray-500", "{detail}" }
            }
        }
    }
}
