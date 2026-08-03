use dioxus::prelude::*;

use crate::api::{ApiClient, GpuSnapshot};
use crate::hooks::{use_page_visible, use_poll_tick_gated};
use crate::state::investigation::{set_training_step_context, INVESTIGATION_CONTEXT};

use super::super::components::{
    EvidenceMetric, EvidenceSection, EvidenceSurface, LoadingPanel, UnavailablePanel, WorkspacePage,
};
use super::super::model::{format_duration, format_percent, GpuHealth, StepHealth, StepTrendPoint};
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
    let steps = use_resource(move || {
        let _ = refresh_key();
        async move { ApiClient::new().fetch_step_matrix(256, false).await }
    });
    let gpu = use_resource(move || {
        let _ = refresh_key();
        async move { ApiClient::new().fetch_gpu_latest().await }
    });
    let step_state = steps.read().clone();
    let gpu_state = gpu.read().clone();
    let step_health = step_state
        .as_ref()
        .and_then(|result| result.as_ref().ok())
        .map(StepHealth::from_matrix);
    let gpu_health = gpu_state
        .as_ref()
        .and_then(|result| result.as_ref().ok())
        .map(|snapshots| GpuHealth::from_snapshots(snapshots));
    let poll_label = if *DASHBOARD_AUTO_REFRESH.read() {
        format!("Live · {}s", POLL_MS / 1000)
    } else {
        "Manual refresh".to_string()
    };

    rsx! {
        WorkspacePage {
            title: "Dashboard".to_string(),
            subtitle: "Latest step and accelerator samples from this node.".to_string(),
            actions: rsx! {
                span { class: "text-xs text-gray-500", "{poll_label}" }
            },

            EvidenceSurface {
                div { class: "grid items-start xl:grid-cols-2",
                    EvidenceSection {
                        title: "Step time".to_string(),
                        match step_state.as_ref() {
                            None => rsx! { LoadingPanel { label: "Loading training steps".to_string() } },
                            Some(Err(error)) => rsx! { UnavailablePanel {
                                label: "Step samples unavailable".to_string(),
                                detail: error.display_message(),
                            }},
                            Some(Ok(matrix)) if matrix.samples.is_empty() => rsx! { UnavailablePanel {
                                label: "No train.step spans yet".to_string(),
                                detail: "No completed step sample was returned.".to_string(),
                            }},
                            Some(Ok(_)) => rsx! { StepTimePanel { health: step_health.clone().unwrap_or_default() } },
                        }
                    }
                    div { class: "border-t border-gray-200 xl:border-l xl:border-t-0",
                        EvidenceSection {
                            title: "GPU load".to_string(),
                            match gpu_state.as_ref() {
                                None => rsx! { LoadingPanel { label: "Loading GPU samples".to_string() } },
                                Some(Err(error)) => rsx! { UnavailablePanel {
                                    label: "GPU samples unavailable".to_string(),
                                    detail: error.display_message(),
                                }},
                                Some(Ok(snapshots)) if snapshots.is_empty() => rsx! { UnavailablePanel {
                                    label: "No GPU samples".to_string(),
                                    detail: "The latest utilization query returned no devices.".to_string(),
                                }},
                                Some(Ok(snapshots)) => rsx! { GpuLoadPanel {
                                    snapshots: snapshots.clone(),
                                    health: gpu_health.clone().unwrap_or_default(),
                                }},
                            }
                        }
                    }
                }
                EvidenceSection {
                    title: "Latest rank step time".to_string(),
                    divided: true,
                    match step_state.as_ref() {
                        None => rsx! { LoadingPanel { label: "Loading rank samples".to_string() } },
                        Some(Err(error)) => rsx! { UnavailablePanel {
                            label: "Rank samples unavailable".to_string(),
                            detail: error.display_message(),
                        }},
                        Some(Ok(matrix)) if matrix.samples.is_empty() => rsx! { UnavailablePanel {
                            label: "No comparable rank samples".to_string(),
                            detail: "No completed step sample was returned.".to_string(),
                        }},
                        Some(Ok(_)) => rsx! { RankLatencyPanel { health: step_health.clone().unwrap_or_default() } },
                    }
                }
            }
        }
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
                EvidenceMetric { label: "Latest step", value: latest_step, detail: None }
                EvidenceMetric { label: "Median", value: format_duration(health.median_ms), detail: None }
                EvidenceMetric { label: "P95", value: format_duration(health.p95_ms), detail: None }
                EvidenceMetric { label: "Maximum", value: maximum, detail: maximum_detail }
            }
            StepTrendChart { points: health.trend }
        }
    }
}

#[component]
pub(super) fn StepTrendChart(points: Vec<StepTrendPoint>) -> Element {
    if points.is_empty() {
        return rsx! {
            UnavailablePanel {
                label: "No step trend".to_string(),
                detail: "Only the latest rank samples were returned.".to_string(),
            }
        };
    }

    let width = 720.0;
    let height = 178.0;
    let pad_left = 50.0;
    let pad_right = 12.0;
    let pad_top = 10.0;
    let pad_bottom = 26.0;
    let plot_width = width - pad_left - pad_right;
    let plot_height = height - pad_top - pad_bottom;
    let minimum = points
        .iter()
        .map(|point| point.median_ms)
        .fold(f64::INFINITY, f64::min);
    let maximum = points
        .iter()
        .map(|point| point.p95_ms)
        .fold(f64::NEG_INFINITY, f64::max);
    let padding = ((maximum - minimum) * 0.08)
        .max(maximum.abs() * 0.02)
        .max(0.1);
    let y_min = (minimum - padding).max(0.0);
    let y_max = maximum + padding;
    let y_span = (y_max - y_min).max(0.1);
    let x_span = points.len().saturating_sub(1).max(1) as f64;
    let coordinates = |value: fn(&StepTrendPoint) -> f64| {
        points
            .iter()
            .enumerate()
            .map(|(index, point)| {
                let x = pad_left + index as f64 / x_span * plot_width;
                let y = pad_top + (y_max - value(point)) / y_span * plot_height;
                format!("{x:.1},{y:.1}")
            })
            .collect::<Vec<_>>()
            .join(" ")
    };
    let median_line = coordinates(|point| point.median_ms);
    let p95_line = coordinates(|point| point.p95_ms);
    let first_step = points.first().map(|point| point.step).unwrap_or_default();
    let last_step = points.last().map(|point| point.step).unwrap_or_default();

    rsx! {
        div {
            class: "border-t border-gray-100 pt-3",
            role: "img",
            aria_label: "Recent step duration trend from step {first_step} to {last_step}; blue is median and violet is P95",
            div { class: "mb-1 flex items-center justify-between text-xs text-gray-500",
                span { "Recent steps" }
                div { class: "flex gap-3",
                    span { class: "flex items-center gap-1",
                        span { class: "inline-block h-px w-4 bg-blue-600" }
                        "median"
                    }
                    span { class: "flex items-center gap-1",
                        span { class: "inline-block h-px w-4 bg-violet-500" }
                        "P95"
                    }
                }
            }
            svg {
                class: "h-44 w-full",
                view_box: "0 0 {width} {height}",
                preserve_aspect_ratio: "none",
                for tick in 0..=3 {
                    {
                        let ratio = tick as f64 / 3.0;
                        let y = pad_top + ratio * plot_height;
                        let value = y_max - ratio * y_span;
                        rsx! {
                            line {
                                x1: "{pad_left}", y1: "{y}",
                                x2: "{pad_left + plot_width}", y2: "{y}",
                                stroke: "#e5e7eb", stroke_width: "1",
                            }
                            text {
                                x: "{pad_left - 6.0}", y: "{y + 3.0}",
                                text_anchor: "end", font_size: "11", fill: "#6b7280",
                                "{format_duration(Some(value))}"
                            }
                        }
                    }
                }
                polyline {
                    points: "{p95_line}", fill: "none", stroke: "#8b5cf6",
                    stroke_width: "2", stroke_linejoin: "round", stroke_linecap: "round",
                    vector_effect: "non-scaling-stroke",
                }
                polyline {
                    points: "{median_line}", fill: "none", stroke: "#2563eb",
                    stroke_width: "2.5", stroke_linejoin: "round", stroke_linecap: "round",
                    vector_effect: "non-scaling-stroke",
                }
                text {
                    x: "{pad_left}", y: "{height - 5.0}", text_anchor: "start",
                    font_size: "11", fill: "#6b7280", "step {first_step}"
                }
                text {
                    x: "{pad_left + plot_width}", y: "{height - 5.0}", text_anchor: "end",
                    font_size: "11", fill: "#6b7280", "step {last_step}"
                }
            }
        }
    }
}

#[component]
fn RankLatencyPanel(health: StepHealth) -> Element {
    let coverage = format!("{} / {}", health.observed_ranks, health.expected_ranks);
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
                EvidenceMetric { label: "Ranks", value: coverage, detail: None }
                EvidenceMetric { label: "Median", value: format_duration(health.median_ms), detail: None }
                EvidenceMetric { label: "P95", value: format_duration(health.p95_ms), detail: None }
                EvidenceMetric { label: "Maximum rank", value: slowest, detail: slowest_detail }
            }
            if !health.nodes_failed.is_empty() {
                details {
                    class: "rounded-md border border-amber-200 bg-amber-50 px-3 py-2 text-xs text-amber-900",
                    summary {
                        class: "cursor-pointer font-medium focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-amber-600 focus-visible:ring-offset-2",
                        "{health.nodes_failed.len()} node(s) did not return step samples · show nodes"
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

    rsx! {
        div {
            class: "grid grid-cols-[3.5rem_minmax(0,1fr)_3.5rem_minmax(0,1fr)_3.5rem] items-center gap-2",
            title: "{snapshot.name} · {snapshot.backend}",
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
