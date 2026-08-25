use dioxus::prelude::*;

use crate::api::{ApiClient, GpuSnapshot, StepMatrixResponse};
use crate::hooks::{use_page_visible, use_poll_tick_gated};
use crate::state::investigation::{
    set_memory_device_context, set_training_step_context, INVESTIGATION_CONTEXT,
};
use crate::state::page_context::publish_page_evidence;

use super::super::capabilities::{capability_status, CapabilityStatus};
use super::super::components::{
    EvidenceLink, EvidenceMetric, EvidenceSection, EvidenceSurface, InlineNotice, LoadingPanel,
    NoticeTone, UnavailablePanel, WorkspacePage,
};
use super::super::evidence::{
    step_matrix_payload, EvidenceBundle, EvidencePayload, EvidenceReceipt, EvidenceRequest,
    EvidenceScope,
};
use super::super::model::{format_duration, format_percent, GpuHealth, StepHealth, StepTrendPoint};
use super::super::routes::NextRoute;
use super::super::settings::{DASHBOARD_AUTO_REFRESH, DASHBOARD_MANUAL_REFRESH};

const POLL_MS: u32 = 5_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StepPresentation {
    Loading,
    Ready,
    NotReported,
    Empty,
    Failed,
}

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
    let step_request = use_memo(move || {
        EvidenceRequest::new(
            u64::from(refresh_key()),
            EvidenceScope::ClusterFanout,
            None,
            INVESTIGATION_CONTEXT.read().clone(),
        )
    });
    let gpu_request = use_memo(move || {
        EvidenceRequest::new(
            u64::from(refresh_key()),
            EvidenceScope::LocalProcess,
            None,
            INVESTIGATION_CONTEXT.read().clone(),
        )
    });
    let steps = use_resource(move || {
        let request = step_request();
        let available = capability_status(
            "python",
            "trace_event",
            &["record_type", "span_id", "name", "time"],
        );
        async move {
            let matrix = if available.allows_query() {
                ApiClient::new().fetch_step_matrix(256, true).await
            } else {
                Ok(empty_step_matrix())
            }?;
            Ok(step_matrix_payload(matrix, &request))
        }
    });
    let gpu = use_resource(move || {
        let request = gpu_request();
        let available = capability_status(
            "gpu",
            "utilization",
            &["device_id", "used_bytes", "total_bytes"],
        );
        async move {
            let snapshots = if available.allows_query() {
                ApiClient::new().fetch_gpu_latest().await?
            } else {
                Vec::new()
            };
            let receipt = EvidenceReceipt::local("gpu.utilization", &request, snapshots.len());
            Ok(EvidencePayload::new(snapshots, receipt))
        }
    });
    let step_state = steps.read().clone();
    let gpu_state = gpu.read().clone();
    let step_health = step_state
        .as_ref()
        .and_then(|result| result.as_ref().ok())
        .map(|payload| StepHealth::from_matrix(&payload.value));
    let gpu_health = gpu_state
        .as_ref()
        .and_then(|result| result.as_ref().ok())
        .map(|payload| GpuHealth::from_snapshots(&payload.value));
    let step_matrix = step_state
        .as_ref()
        .and_then(|result| result.as_ref().ok())
        .map(|payload| &payload.value);
    let step_scope = step_scope_label(step_matrix);
    let step_partial = step_matrix.is_some_and(|matrix| matrix.partial);
    let poll_label = if *DASHBOARD_AUTO_REFRESH.read() {
        format!("Live · {}s", POLL_MS / 1000)
    } else {
        "Manual refresh".to_string()
    };
    let step_source = capability_status(
        "python",
        "trace_event",
        &["record_type", "span_id", "name", "time"],
    );
    let gpu_source = capability_status(
        "gpu",
        "utilization",
        &["device_id", "used_bytes", "total_bytes"],
    );
    let gpu_reported = gpu_source != CapabilityStatus::Missing;
    let step_presentation = dashboard_step_presentation(step_source, step_state.as_ref());
    let bundle_request = step_request();
    let bundle_step_state = step_state.clone();
    let bundle_gpu_state = gpu_state.clone();
    use_effect(move || {
        if let Some(snapshot) = dashboard_evidence_bundle(
            &bundle_request,
            bundle_step_state.as_ref(),
            bundle_gpu_state.as_ref(),
        ) {
            publish_page_evidence(
                "dashboard",
                &crate::state::investigation::investigation_context_key(&bundle_request.context),
                bundle_request.requested_at_ms,
                snapshot,
            );
        }
    });
    let step_notice = match step_presentation {
        StepPresentation::NotReported => Some((
            "Training step source not reported".to_string(),
            "Dashboard is showing process-local accelerator evidence only.".to_string(),
            NoticeTone::Info,
        )),
        StepPresentation::Empty => Some((
            "No completed training steps".to_string(),
            "The cluster step request completed but returned no train.step samples.".to_string(),
            NoticeTone::Info,
        )),
        StepPresentation::Failed => Some((
            "Training step evidence unavailable".to_string(),
            step_state
                .as_ref()
                .and_then(|result| result.as_ref().err())
                .map(|error| error.display_message())
                .unwrap_or_else(|| "The cluster step request failed.".to_string()),
            NoticeTone::Warning,
        )),
        StepPresentation::Loading | StepPresentation::Ready => None,
    };

    rsx! {
        WorkspacePage {
            title: "Dashboard".to_string(),
            subtitle: "Each panel states its collection scope; cluster and process-local values are not combined.".to_string(),
            actions: rsx! {
                span { class: "text-xs text-gray-500", "{poll_label}" }
            },

            if let Some((title, detail, tone)) = step_notice {
                InlineNotice {
                    title,
                    detail,
                    tone,
                }
            }

            EvidenceSurface {
                div { class: if gpu_reported && matches!(step_presentation, StepPresentation::Ready | StepPresentation::Loading) { "grid items-start xl:grid-cols-2" } else { "grid items-start" },
                    if step_presentation == StepPresentation::Ready {
                        EvidenceSection {
                        title: "Cluster step time".to_string(),
                        subtitle: Some("Summary metrics use the latest completed step; the chart shows recent cluster history.".to_string()),
                        actions: rsx! {
                            ScopeBadge { label: step_scope.clone() }
                            if step_partial {
                                span { class: "rounded-full border border-amber-200 bg-amber-50 px-2 py-0.5 text-xs font-medium text-amber-800", "Partial" }
                            }
                        },
                        StepTimePanel { health: step_health.clone().unwrap_or_default() }
                        }
                    } else if step_presentation == StepPresentation::Loading {
                        EvidenceSection {
                            title: "Cluster step evidence".to_string(),
                            subtitle: Some("One request supplies both the trend and rank comparison below.".to_string()),
                            LoadingPanel { label: "Loading training steps".to_string() }
                        }
                    }
                    if gpu_reported {
                        div { class: if matches!(step_presentation, StepPresentation::Ready | StepPresentation::Loading) { "border-t border-gray-200 xl:border-l xl:border-t-0" } else { "" },
                            EvidenceSection {
                                title: "Local GPU load".to_string(),
                                subtitle: Some("Latest accelerator samples exposed by the current server process only.".to_string()),
                                actions: rsx! {
                                    ScopeBadge { label: "Process-local".to_string() }
                                    EvidenceLink { route: NextRoute::Memory {}, label: "Open Memory →".to_string() }
                                },
                                match gpu_state.as_ref() {
                                    None => rsx! { LoadingPanel { label: "Loading GPU samples".to_string() } },
                                    Some(Err(error)) => rsx! { UnavailablePanel {
                                        label: "GPU samples unavailable".to_string(),
                                        detail: error.display_message(),
                                    }},
                                    Some(Ok(payload)) if payload.value.is_empty() => rsx! { UnavailablePanel {
                                        label: "No GPU samples".to_string(),
                                        detail: "The GPU collector is active but has not returned a device sample yet.".to_string(),
                                    }},
                                    Some(Ok(payload)) => rsx! { GpuLoadPanel {
                                        snapshots: payload.value.clone(),
                                        health: gpu_health.clone().unwrap_or_default(),
                                    }},
                                }
                            }
                        }
                    }
                }
                if !gpu_reported {
                    div { class: "flex flex-wrap items-center justify-between gap-2 border-t border-gray-200 px-4 py-2 text-xs text-gray-600",
                        span { "Local GPU metrics are not enabled for this process." }
                        EvidenceLink { route: NextRoute::Memory {}, label: "Open Memory →".to_string() }
                    }
                }
                if step_presentation == StepPresentation::Ready {
                    EvidenceSection {
                        title: "Cluster rank step time".to_string(),
                        subtitle: Some("Latest completed train.step duration for each rank present in the cluster response.".to_string()),
                        actions: rsx! {
                            ScopeBadge { label: step_scope.clone() }
                            if step_partial {
                                span { class: "rounded-full border border-amber-200 bg-amber-50 px-2 py-0.5 text-xs font-medium text-amber-800", "Partial" }
                            }
                        },
                        divided: true,
                        RankLatencyPanel { health: step_health.clone().unwrap_or_default() }
                    }
                }
            }
        }
    }
}

fn dashboard_step_presentation(
    source: CapabilityStatus,
    state: Option<&crate::utils::error::Result<EvidencePayload<StepMatrixResponse>>>,
) -> StepPresentation {
    match source {
        CapabilityStatus::Checking => StepPresentation::Loading,
        CapabilityStatus::Missing => StepPresentation::NotReported,
        CapabilityStatus::Available | CapabilityStatus::CatalogUnavailable => match state {
            None => StepPresentation::Loading,
            Some(Err(_)) => StepPresentation::Failed,
            Some(Ok(payload)) if payload.value.samples.is_empty() => StepPresentation::Empty,
            Some(Ok(_)) => StepPresentation::Ready,
        },
    }
}

fn dashboard_evidence_bundle(
    request: &EvidenceRequest,
    steps: Option<&crate::utils::error::Result<EvidencePayload<StepMatrixResponse>>>,
    gpu: Option<&crate::utils::error::Result<EvidencePayload<Vec<GpuSnapshot>>>>,
) -> Option<String> {
    let (steps, gpu) = (steps?, gpu?);
    let mut bundle = EvidenceBundle::new("dashboard", request);
    match steps {
        Ok(payload) => bundle.push(
            &payload.receipt,
            super::super::page_snapshot::format_step_matrix(&payload.value, request),
        ),
        Err(error) => bundle.push_failure("train.step", &error.display_message()),
    }
    match gpu {
        Ok(payload) => bundle.push(&payload.receipt, gpu_snapshot_preview(&payload.value)),
        Err(error) => bundle.push_failure("gpu.utilization", &error.display_message()),
    }
    Some(bundle.render())
}

fn gpu_snapshot_preview(snapshots: &[GpuSnapshot]) -> String {
    if snapshots.is_empty() {
        return "(empty)".into();
    }
    snapshots
        .iter()
        .map(|snapshot| {
            format!(
                "GPU {} · memory {:.1}% · compute {}",
                snapshot.device_id,
                snapshot.mem_used_pct,
                snapshot
                    .gpu_util_pct
                    .map(|value| format!("{value:.1}%"))
                    .unwrap_or_else(|| "not reported".into()),
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn empty_step_matrix() -> StepMatrixResponse {
    StepMatrixResponse {
        samples: Vec::new(),
        rank_count: 0,
        step_count: 0,
        cluster: true,
        partial: false,
        nodes_queried: 0,
        nodes_failed: Vec::new(),
    }
}

#[component]
fn ScopeBadge(label: String) -> Element {
    rsx! {
        span {
            class: "rounded-full border border-gray-200 bg-gray-50 px-2 py-0.5 text-xs font-medium text-gray-700",
            aria_label: "Evidence scope: {label}",
            "{label}"
        }
    }
}

fn step_scope_label(matrix: Option<&StepMatrixResponse>) -> String {
    match matrix {
        Some(matrix) if matrix.cluster => format!("Cluster · {} observed ranks", matrix.rank_count),
        Some(_) => "Process-local response".to_string(),
        None => "Cluster requested".to_string(),
    }
}

#[component]
fn StepTimePanel(health: StepHealth) -> Element {
    let latest_step = health
        .latest_step
        .map(|step| step.to_string())
        .unwrap_or_else(|| "—".to_string());
    let maximum = health
        .slowest_ms
        .map(|duration| format_duration(Some(duration)))
        .unwrap_or_else(|| "—".to_string());
    let maximum_detail = health.slowest_rank.map(|rank| format!("rank {rank}"));

    rsx! {
        div { class: "space-y-4",
            div { class: "grid grid-cols-4 divide-x divide-gray-200",
                EvidenceMetric { label: "Latest complete step", value: latest_step, detail: None }
                EvidenceMetric { label: "Rank median", value: format_duration(health.median_ms), detail: None }
                EvidenceMetric { label: "Rank P95", value: format_duration(health.p95_ms), detail: None }
                EvidenceMetric { label: "Slowest rank", value: maximum, detail: maximum_detail }
            }
            StepTrendChart { points: health.trend }
        }
    }
}

#[component]
pub(super) fn StepTrendChart(points: Vec<StepTrendPoint>) -> Element {
    let mut hide_first = use_signal(|| false);
    if points.is_empty() {
        return rsx! {
            UnavailablePanel {
                label: "No step trend".to_string(),
                detail: "Only the latest rank samples were returned.".to_string(),
            }
        };
    }

    let can_hide_first = points.len() > 3;
    let plotted = if can_hide_first && hide_first() {
        points[1..].to_vec()
    } else {
        points
    };
    let width = 1000.0;
    let height = 100.0;
    let minimum = plotted
        .iter()
        .map(|point| point.median_ms)
        .fold(f64::INFINITY, f64::min);
    let maximum = plotted
        .iter()
        .map(|point| point.p95_ms)
        .fold(f64::NEG_INFINITY, f64::max);
    let padding = ((maximum - minimum) * 0.08)
        .max(maximum.abs() * 0.02)
        .max(0.1);
    let y_min = (minimum - padding).max(0.0);
    let y_max = maximum + padding;
    let y_span = (y_max - y_min).max(0.1);
    let point_count = plotted.len().max(1) as f64;
    let coordinates = |value: fn(&StepTrendPoint) -> f64| {
        plotted
            .iter()
            .enumerate()
            .map(|(index, point)| {
                let x = (index as f64 + 0.5) / point_count * width;
                let y = (y_max - value(point)) / y_span * height;
                format!("{x:.1},{y:.1}")
            })
            .collect::<Vec<_>>()
            .join(" ")
    };
    let median_line = coordinates(|point| point.median_ms);
    let p95_line = coordinates(|point| point.p95_ms);
    let first_step = plotted.first().map(|point| point.step).unwrap_or_default();
    let last_step = plotted.last().map(|point| point.step).unwrap_or_default();
    let visible_ticks = step_tick_indices(plotted.len());
    let step_ticks = plotted
        .iter()
        .enumerate()
        .map(|(index, point)| visible_ticks.contains(&index).then_some(point.step))
        .collect::<Vec<_>>();
    let step_columns = step_ticks.len().max(1);

    rsx! {
        div {
            class: "border-t border-gray-100 pt-3",
            role: "img",
            aria_label: "Recent step duration trend from step {first_step} to {last_step}; blue is median and violet is P95",
            div { class: "mb-1 flex items-center justify-between text-xs text-gray-500",
                div { class: "flex items-center gap-2",
                    span { "Recent steps" }
                    if can_hide_first {
                        button {
                            r#type: "button",
                            class: "rounded border border-gray-200 px-1.5 py-0.5 font-medium text-gray-600 hover:border-gray-300 hover:bg-gray-50",
                            aria_pressed: hide_first().to_string(),
                            onclick: move |_| hide_first.toggle(),
                            if hide_first() { "Show warmup" } else { "Hide warmup" }
                        }
                    }
                }
                div { class: "flex gap-3",
                    span { class: "flex items-center gap-1",
                        span { class: "inline-block h-px w-4 bg-blue-600" }
                        "Rank median"
                    }
                    span { class: "flex items-center gap-1",
                        span { class: "inline-block h-px w-4 bg-violet-500" }
                        "Rank P95"
                    }
                }
            }
            div { class: "relative h-44",
                div { class: "absolute bottom-6 left-0 top-0 w-14",
                    for tick in 0..=3 {
                        {
                            let ratio = tick as f64 / 3.0;
                            let value = y_max - ratio * y_span;
                            let position = if tick == 0 || tick == 3 {
                                String::new()
                            } else {
                                format!("top: {:.2}%;", ratio * 100.0)
                            };
                            rsx! {
                                span {
                                    class: if tick == 0 {
                                        "absolute left-0 top-0 w-14 whitespace-nowrap text-right text-xs text-gray-500"
                                    } else if tick == 3 {
                                        "absolute bottom-0 left-0 w-14 whitespace-nowrap text-right text-xs text-gray-500"
                                    } else {
                                        "absolute left-0 w-14 -translate-y-1/2 whitespace-nowrap text-right text-xs text-gray-500"
                                    },
                                    style: "{position}",
                                    "{format_duration(Some(value))}"
                                }
                            }
                        }
                    }
                }
                div { class: "absolute bottom-6 left-16 right-3 top-0",
                    svg {
                        class: "h-full w-full",
                        view_box: "0 0 {width} {height}",
                        preserve_aspect_ratio: "none",
                        for tick in 0..=3 {
                            {
                                let y = tick as f64 / 3.0 * height;
                                rsx! {
                                    line {
                                        x1: "0", y1: "{y}",
                                        x2: "{width}", y2: "{y}",
                                        stroke: "#e5e7eb", stroke_width: "1",
                                        vector_effect: "non-scaling-stroke",
                                    }
                                }
                            }
                        }
                        polyline {
                            points: "{p95_line}", fill: "none", stroke: "#8b5cf6",
                            stroke_width: "4", stroke_opacity: "0.65",
                            stroke_linejoin: "round", stroke_linecap: "round",
                            vector_effect: "non-scaling-stroke",
                        }
                        polyline {
                            points: "{median_line}", fill: "none", stroke: "#2563eb",
                            stroke_width: "2", stroke_linejoin: "round", stroke_linecap: "round",
                            vector_effect: "non-scaling-stroke",
                        }
                    }
                }
                span { class: "absolute bottom-0 left-0 w-14 text-right text-xs text-gray-500", "Step" }
                div {
                    class: "absolute bottom-0 grid h-4 items-start",
                    style: "left: 4rem; width: calc(100% - 4.75rem); grid-template-columns: repeat({step_columns}, minmax(0, 1fr));",
                    for step in step_ticks {
                        span {
                            class: "min-w-0 truncate text-center font-mono text-xs text-gray-500",
                            if let Some(step) = step {
                                "{step}"
                            }
                        }
                    }
                }
            }
        }
    }
}

fn step_tick_indices(point_count: usize) -> Vec<usize> {
    const MAX_TICKS: usize = 10;
    if point_count <= MAX_TICKS {
        return (0..point_count).collect();
    }
    let interval = point_count.div_ceil(MAX_TICKS);
    let first = (point_count - 1) % interval;
    (first..point_count).step_by(interval).collect()
}

#[component]
fn RankLatencyPanel(health: StepHealth) -> Element {
    let slowest = health
        .slowest_rank
        .map(|rank| format!("R{rank}"))
        .unwrap_or_else(|| "—".to_string());
    let slowest_detail = health
        .slowest_ratio()
        .map(|ratio| format!("{ratio:.2}× median"));
    let shown = health.rank_durations.len().min(12);
    let failed_nodes_title = health.nodes_failed.join(", ");

    rsx! {
        div { class: "space-y-4",
            div { class: "grid grid-cols-4 divide-x divide-gray-200",
                EvidenceMetric { label: "Observed ranks", value: health.observed_ranks.to_string(), detail: None }
                EvidenceMetric { label: "Median", value: format_duration(health.median_ms), detail: None }
                EvidenceMetric { label: "P95", value: format_duration(health.p95_ms), detail: None }
                EvidenceMetric { label: "Maximum rank", value: slowest, detail: slowest_detail }
            }
            if !health.nodes_failed.is_empty() {
                details {
                    class: "rounded-md border border-amber-200 bg-amber-50 px-3 py-2 text-xs text-amber-900",
                    summary {
                        class: "cursor-pointer font-medium focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-amber-600 focus-visible:ring-offset-2",
                        "{health.nodes_failed.len()} peer endpoint(s) did not return step samples · show peers"
                    }
                    p { class: "mt-1 break-all font-mono text-xs", "{failed_nodes_title}" }
                }
            }
            div { class: "flex items-center justify-between gap-2 border-t border-gray-100 pt-3 text-xs text-gray-500",
                span { "Slowest {shown} of {health.rank_durations.len()} observed ranks" }
                span { class: "flex items-center gap-1",
                    span { class: "inline-block h-3 border-l border-dashed border-gray-500" }
                    "median"
                }
            }
            RankLatencyBars { health }
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
            for (rank, duration) in health.rank_durations.iter().take(12) {
                RankLatencyBar {
                    rank: *rank,
                    duration: *duration,
                    maximum,
                    median: health.median_ms,
                    slowest: Some(*rank) == health.slowest_rank,
                    local_step: health.latest_step,
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
fn RankLatencyBar(
    rank: i32,
    duration: f64,
    maximum: f64,
    median: Option<f64>,
    slowest: bool,
    local_step: Option<i64>,
) -> Element {
    let width_style = format!(
        "width: {:.1}%;",
        (duration / maximum * 100.0).clamp(2.0, 100.0)
    );
    let median_style = format!(
        "left: {:.1}%;",
        (median.unwrap_or_default() / maximum * 100.0).clamp(0.0, 100.0)
    );
    let pin_label = format!(
        "Pin rank {rank} at step {}",
        local_step
            .map(|step| step.to_string())
            .unwrap_or_else(|| "latest".to_string())
    );
    let pinned = INVESTIGATION_CONTEXT.read().rank == Some(rank)
        && INVESTIGATION_CONTEXT.read().local_step == local_step;
    let row_class = if pinned {
        "grid w-full grid-cols-[6.5rem_minmax(0,1fr)_5.5rem] items-center gap-3 rounded bg-blue-50 px-1 py-0.5 text-left ring-1 ring-blue-200 focus:outline-none focus:ring-2 focus:ring-blue-500/30"
    } else {
        "grid w-full grid-cols-[6.5rem_minmax(0,1fr)_5.5rem] items-center gap-3 rounded px-1 py-0.5 text-left hover:bg-blue-50 focus:outline-none focus:ring-2 focus:ring-blue-500/30"
    };

    rsx! {
        button {
            r#type: "button",
            class: "{row_class}",
            aria_label: "{pin_label}",
            aria_pressed: pinned.to_string(),
            onclick: move |_| set_training_step_context(rank, local_step, None),
            div { class: "flex items-center gap-1 text-xs font-medium text-gray-700",
                "rank {rank}"
                if slowest {
                    span { class: "rounded bg-violet-100 px-1 text-violet-800", "max" }
                }
                if pinned {
                    span { class: "font-bold text-blue-700", aria_hidden: "true", "✓" }
                }
            }
            div { class: "relative h-2.5 rounded-full bg-gray-100",
                div {
                    class: if slowest { "h-full rounded-full bg-violet-500" } else { "h-full rounded-full bg-blue-500" },
                    style: "{width_style}",
                }
                span {
                    class: "absolute inset-y-[-2px] border-l border-dashed border-gray-600",
                    style: "{median_style}",
                }
            }
            div { class: "text-right text-xs tabular-nums text-gray-600", "{format_duration(Some(duration))}" }
        }
    }
}

#[component]
fn GpuLoadPanel(snapshots: Vec<GpuSnapshot>, health: GpuHealth) -> Element {
    rsx! {
        div { class: "space-y-4",
            div { class: "grid grid-cols-3 divide-x divide-gray-200",
                EvidenceMetric { label: "Devices", value: health.device_count.to_string(), detail: None }
                EvidenceMetric { label: "Average util", value: format_percent(health.average_util_pct), detail: None }
                EvidenceMetric { label: "Average memory", value: format_percent(health.average_memory_pct), detail: None }
            }
            div { class: "space-y-2 border-t border-gray-100 pt-3",
                div { class: "grid grid-cols-[3.5rem_minmax(0,1fr)_3.5rem_minmax(0,1fr)_3.5rem] gap-2 text-xs uppercase tracking-wide text-gray-500",
                    span { "Device" }
                    span { "Utilization" }
                    span {}
                    span { "Memory" }
                    span {}
                }
                for snapshot in snapshots.iter().take(16) {
                    GpuLoadRow { snapshot: snapshot.clone() }
                }
                if snapshots.len() > 16 {
                    p { class: "text-xs text-gray-500", "Showing 16 of {snapshots.len()} devices" }
                }
            }
        }
    }
}

#[component]
fn GpuLoadRow(snapshot: GpuSnapshot) -> Element {
    let util = snapshot.gpu_util_pct.map(f64::from);
    let memory = f64::from(snapshot.mem_used_pct);
    let util_width = format!("width: {:.1}%;", util.unwrap_or_default().clamp(0.0, 100.0));
    let memory_width = format!("width: {:.1}%;", memory.clamp(0.0, 100.0));
    let device_label = format!("GPU {}", snapshot.device_id);
    let device_id = snapshot.device_id;
    let selected = INVESTIGATION_CONTEXT.read().device_id == Some(device_id);

    rsx! {
        button {
            r#type: "button",
            class: if selected {
                "grid w-full grid-cols-[3.5rem_minmax(0,1fr)_3.5rem_minmax(0,1fr)_3.5rem] items-center gap-2 rounded bg-blue-50 px-1 py-1 text-left ring-1 ring-blue-200"
            } else {
                "grid w-full grid-cols-[3.5rem_minmax(0,1fr)_3.5rem_minmax(0,1fr)_3.5rem] items-center gap-2 rounded px-1 py-1 text-left hover:bg-blue-50/60 focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-500"
            },
            title: "{snapshot.name} · {snapshot.backend}",
            aria_label: "Select {device_label} for Memory evidence",
            aria_pressed: selected.to_string(),
            onclick: move |_| set_memory_device_context(None, None, device_id),
            span { class: "truncate font-mono text-xs text-gray-600", "{device_label}" }
            div { class: "h-2 rounded-full bg-gray-100",
                div { class: "h-full rounded-full bg-blue-500", style: "{util_width}" }
            }
            span { class: "text-right text-xs tabular-nums text-gray-600", "{format_percent(util)}" }
            div { class: "h-2 rounded-full bg-gray-100",
                div { class: "h-full rounded-full bg-violet-500", style: "{memory_width}" }
            }
            span { class: "text-right text-xs tabular-nums text-gray-600", "{format_percent(Some(memory))}" }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matrix(cluster: bool, rank_count: usize) -> StepMatrixResponse {
        StepMatrixResponse {
            samples: Vec::new(),
            rank_count,
            step_count: 0,
            cluster,
            partial: false,
            nodes_queried: 8,
            nodes_failed: Vec::new(),
        }
    }

    #[test]
    fn scope_label_follows_the_returned_response_scope() {
        let cluster = matrix(true, 64);
        let local = matrix(false, 1);

        assert_eq!(
            step_scope_label(Some(&cluster)),
            "Cluster · 64 observed ranks"
        );
        assert_eq!(step_scope_label(Some(&local)), "Process-local response");
        assert_eq!(step_scope_label(None), "Cluster requested");
    }

    #[test]
    fn one_step_source_state_drives_both_dashboard_views() {
        let request =
            EvidenceRequest::new(1, EvidenceScope::ClusterFanout, None, Default::default());
        let empty = Ok(step_matrix_payload(matrix(true, 0), &request));
        let failed = Err(crate::utils::error::AppError::Api("fan-out failed".into()));

        assert_eq!(
            dashboard_step_presentation(CapabilityStatus::Missing, None),
            StepPresentation::NotReported
        );
        assert_eq!(
            dashboard_step_presentation(CapabilityStatus::Available, Some(&empty)),
            StepPresentation::Empty
        );
        assert_eq!(
            dashboard_step_presentation(CapabilityStatus::Available, Some(&failed)),
            StepPresentation::Failed
        );
    }

    #[test]
    fn step_ticks_adapt_integer_interval_to_total_steps() {
        assert_eq!(step_tick_indices(4), vec![0, 1, 2, 3]);
        assert_eq!(step_tick_indices(10), (0..10).collect::<Vec<_>>());
        assert_eq!(
            step_tick_indices(20),
            vec![1, 3, 5, 7, 9, 11, 13, 15, 17, 19]
        );
        assert_eq!(
            step_tick_indices(40),
            vec![3, 7, 11, 15, 19, 23, 27, 31, 35, 39]
        );
        assert_eq!(step_tick_indices(21), vec![2, 5, 8, 11, 14, 17, 20]);
    }
}
