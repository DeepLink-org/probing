use std::collections::{BTreeMap, BTreeSet};

use dioxus::prelude::*;
use probing_proto::prelude::{DataFrame, Ele, Node};

use crate::api::ApiClient;
use crate::components::dataframe_view::DataFrameView;
use crate::hooks::{use_page_visible, use_poll_tick_gated};
use crate::state::investigation::{
    set_memory_device_context, InvestigationContext, INVESTIGATION_CONTEXT,
};
use crate::state::page_context::publish_page_evidence;
use crate::utils::error::Result;

use super::super::capabilities::{capability_status, CapabilityStatus};
use super::super::components::{
    EvidenceMetric, EvidenceSection, EvidenceSurface, InlineNotice, LoadingPanel, NoticeTone,
    UnavailablePanel, WorkspacePage,
};
use super::super::evidence::{
    dataframe_preview, ContextMatch, EvidenceBundle, EvidencePayload, EvidenceReceipt,
    EvidenceRequest, EvidenceScope,
};
use super::super::settings::{MEMORY_CLUSTER_SCOPE, MEMORY_REFRESH, MEMORY_WINDOW_MINUTES};

const POLL_MS: u32 = 5_000;

#[derive(Clone, Debug, PartialEq)]
struct MemoryQuery {
    dataframe: DataFrame,
    receipt: EvidenceReceipt,
}

#[derive(Clone, Debug, PartialEq)]
struct DeviceMemory {
    rank: Option<i32>,
    device_id: i32,
    host: String,
    current_bytes: u64,
    peak_bytes: u64,
    total_bytes: u64,
    sample_count: usize,
}

#[derive(Clone, Debug, PartialEq)]
struct MemoryPoint {
    rank: Option<i32>,
    device_id: i32,
    ts: i64,
    used_bytes: u64,
    total_bytes: u64,
}

#[component]
pub fn MemoryPage() -> Element {
    let visible = use_page_visible();
    let poll = use_poll_tick_gated(POLL_MS, Some(visible));
    let refresh_key = use_memo(move || poll().wrapping_add(*MEMORY_REFRESH.read()));
    let window_minutes = *MEMORY_WINDOW_MINUTES.read();
    let evidence_request = use_memo(move || {
        EvidenceRequest::new(
            u64::from(refresh_key()),
            if *MEMORY_CLUSTER_SCOPE.read() {
                EvidenceScope::ClusterFanout
            } else {
                EvidenceScope::LocalProcess
            },
            Some((*MEMORY_WINDOW_MINUTES.read() as u64).saturating_mul(60_000_000)),
            INVESTIGATION_CONTEXT.read().clone(),
        )
    });

    let nodes = use_resource(move || {
        let request = evidence_request().for_scope(EvidenceScope::ClusterRegistry, None);
        async move {
            let nodes = ApiClient::new().get_nodes().await?;
            let receipt = EvidenceReceipt::local("cluster.nodes", &request, nodes.len());
            Ok(EvidencePayload::new(nodes, receipt))
        }
    });
    let devices = use_resource(move || {
        let request = evidence_request();
        let available = capability_status(
            "gpu",
            "utilization",
            &["device_id", "used_bytes", "total_bytes"],
        );
        async move {
            if available.allows_query() {
                memory_query(
                    &device_summary_sql(request.window_us.unwrap_or_default()),
                    "gpu.utilization.latest",
                    &request,
                )
                .await
            } else {
                Ok(empty_memory_query("gpu.utilization.latest", &request))
            }
        }
    });
    let history = use_resource(move || {
        let request = evidence_request();
        let available = capability_status(
            "gpu",
            "utilization",
            &["device_id", "used_bytes", "total_bytes"],
        );
        async move {
            if available.allows_query() {
                memory_query(
                    &device_history_sql(request.window_us.unwrap_or_default()),
                    "gpu.utilization.history",
                    &request,
                )
                .await
            } else {
                Ok(empty_memory_query("gpu.utilization.history", &request))
            }
        }
    });
    let allocator = use_resource(move || {
        let request = evidence_request();
        let available = capability_status(
            "python",
            "torch_trace",
            &["rank", "allocated", "max_allocated", "cached"],
        );
        async move {
            if available.allows_query() {
                memory_query(ALLOCATOR_SQL, "python.torch_trace.allocator", &request).await
            } else {
                Ok(empty_memory_query("python.torch_trace.allocator", &request))
            }
        }
    });
    let allocation = use_resource(move || {
        let request = evidence_request();
        let available = capability_status(
            "python",
            "torch_trace",
            &["rank", "module", "allocated_delta", "local_step"],
        );
        async move {
            if available.allows_query() {
                memory_query(
                    ALLOCATION_EVIDENCE_SQL,
                    "python.torch_trace.allocation",
                    &request,
                )
                .await
            } else {
                Ok(empty_memory_query(
                    "python.torch_trace.allocation",
                    &request,
                ))
            }
        }
    });

    let node_state = nodes.read().clone();
    let mut device_state = devices.read().clone();
    let history_state = history.read().clone();
    let allocator_state = allocator.read().clone();
    let allocation_state = allocation.read().clone();
    let node_rows = node_state
        .as_ref()
        .and_then(|value| value.as_ref().ok())
        .map(|payload| payload.value.clone())
        .unwrap_or_default();
    let device_rows = device_state
        .as_ref()
        .and_then(|value| value.as_ref().ok())
        .map(|value| parse_devices(&value.dataframe, &node_rows))
        .unwrap_or_default();
    let context = INVESTIGATION_CONTEXT.read().clone();
    let selected = select_device_for_context(&device_rows, &context);
    let context_match = memory_context_match(&evidence_request(), &context, selected.is_some());
    if let Some(Ok(query)) = device_state.as_mut() {
        query.receipt = query.receipt.clone().with_context_match(context_match);
    }
    let bundle_request = evidence_request();
    let bundle_node_state = node_state.clone();
    let bundle_device_state = device_state.clone();
    let bundle_history_state = history_state.clone();
    let bundle_allocator_state = allocator_state.clone();
    let bundle_allocation_state = allocation_state.clone();
    use_effect(move || {
        if let Some(snapshot) = memory_evidence_bundle(
            &bundle_request,
            bundle_node_state.as_ref(),
            bundle_device_state.as_ref(),
            bundle_history_state.as_ref(),
            bundle_allocator_state.as_ref(),
            bundle_allocation_state.as_ref(),
        ) {
            publish_page_evidence(
                "memory",
                &crate::state::investigation::investigation_context_key(&bundle_request.context),
                bundle_request.requested_at_ms,
                snapshot,
            );
        }
    });
    let selection_mismatch = memory_context_requested(&context)
        && matches!(device_state.as_ref(), Some(Ok(_)))
        && !device_rows.is_empty()
        && selected.is_none();
    let current_request = evidence_request();
    let scope = current_request.scope.label();
    let known_ranks = node_rows
        .iter()
        .filter_map(|node| node.rank)
        .collect::<BTreeSet<_>>()
        .len();
    let allocator_source = capability_status(
        "python",
        "torch_trace",
        &["rank", "allocated", "max_allocated", "cached"],
    );
    let device_source = capability_status(
        "gpu",
        "utilization",
        &["device_id", "used_bytes", "total_bytes"],
    );
    let device_reported = device_source != CapabilityStatus::Missing;

    rsx! {
        WorkspacePage {
            title: "Memory".to_string(),
            subtitle: "Physical device capacity, sampled usage, and framework allocator evidence for training and inference.".to_string(),
            actions: rsx! { span { class: "text-xs text-gray-500", "{scope} · {window_minutes}m window · {POLL_MS / 1000}s" } },

            if current_request.scope == EvidenceScope::LocalProcess && known_ranks > 1 {
                InlineNotice {
                    title: format!("Showing 1 of {known_ranks} ranks"),
                    detail: "This page is scoped to the local process. Enable Cluster fan-out to compare memory across every reported rank.".to_string(),
                    tone: NoticeTone::Info,
                }
            }

            if allocator_source == CapabilityStatus::Missing && !device_reported {
                InlineNotice {
                    title: "Memory sources not reported".to_string(),
                    detail: "Neither physical-device sampling nor PyTorch allocator evidence is enabled for this process.".to_string(),
                    tone: NoticeTone::Info,
                }
            } else if allocator_source == CapabilityStatus::Missing {
                InlineNotice {
                    title: "Allocator source not reported".to_string(),
                    detail: "Physical device memory remains available; PyTorch allocator and module-delta sections are hidden.".to_string(),
                    tone: NoticeTone::Info,
                }
            } else if !device_reported {
                InlineNotice {
                    title: "Device memory source not reported".to_string(),
                    detail: "The GPU collector is not enabled for this process; PyTorch allocator evidence remains available.".to_string(),
                    tone: NoticeTone::Info,
                }
            }

            if selection_mismatch {
                InlineNotice {
                    title: "Pinned device not returned in this scope".to_string(),
                    detail: format!(
                        "{} was not present in the {scope} memory result. No fallback device is selected; enable Cluster fan-out or select a reported device.",
                        memory_context_label(&context),
                    ),
                    tone: NoticeTone::Warning,
                }
            }

            EvidenceSurface {
                EvidenceSection {
                    title: "Device memory".to_string(),
                    subtitle: Some("Current usage is the latest reported sample; peak is the highest sample inside the selected window.".to_string()),
                    DeviceOverview { state: device_state.clone(), devices: device_rows.clone(), source_reported: device_reported }
                }
                EvidenceSection {
                    title: "Memory timeline".to_string(),
                    subtitle: Some("The selected physical device only; this curve does not infer a leak from growth alone.".to_string()),
                    divided: true,
                    MemoryTimeline {
                        state: history_state,
                        selected: selected.clone(),
                        window_minutes,
                        source_reported: device_reported,
                        mismatch_detail: selection_mismatch.then(|| format!(
                            "{} has no sample in the {scope} result.",
                            memory_context_label(&context),
                        )),
                    }
                }
                if allocator_source != CapabilityStatus::Missing {
                    EvidenceSection {
                        title: "Allocator by rank".to_string(),
                        subtitle: Some("Latest reported PyTorch allocated, peak allocated, and reserved values.".to_string()),
                        body_class: "p-0".to_string(),
                        divided: true,
                        EvidenceDataFrame { state: allocator_state, loading: "Loading allocator samples", empty: "No framework allocator samples" }
                    }
                    EvidenceSection {
                        title: "Allocation evidence".to_string(),
                        subtitle: Some("Measured positive allocation deltas by module over the latest ten reported steps; ordering is evidence for inspection, not a root-cause claim.".to_string()),
                        body_class: "p-0".to_string(),
                        divided: true,
                        EvidenceDataFrame { state: allocation_state, loading: "Loading module allocation evidence", empty: "No module allocation deltas" }
                    }
                }
            }
        }
    }
}

fn empty_memory_query(source: &'static str, request: &EvidenceRequest) -> MemoryQuery {
    MemoryQuery {
        dataframe: DataFrame::default(),
        receipt: EvidenceReceipt::local(source, request, 0),
    }
}

fn memory_evidence_bundle(
    request: &EvidenceRequest,
    nodes: Option<&Result<EvidencePayload<Vec<Node>>>>,
    devices: Option<&Result<MemoryQuery>>,
    history: Option<&Result<MemoryQuery>>,
    allocator: Option<&Result<MemoryQuery>>,
    allocation: Option<&Result<MemoryQuery>>,
) -> Option<String> {
    let (nodes, devices, history, allocator, allocation) =
        (nodes?, devices?, history?, allocator?, allocation?);
    let mut bundle = EvidenceBundle::new("memory", request);
    match nodes {
        Ok(payload) => bundle.push(
            &payload.receipt,
            super::super::page_snapshot::format_nodes(&payload.value),
        ),
        Err(error) => bundle.push_failure("cluster.nodes", &error.display_message()),
    }
    push_memory_query(&mut bundle, "gpu.utilization.latest", devices, 16);
    push_memory_query(&mut bundle, "gpu.utilization.history", history, 24);
    push_memory_query(&mut bundle, "python.torch_trace.allocator", allocator, 8);
    push_memory_query(&mut bundle, "python.torch_trace.allocation", allocation, 20);
    Some(bundle.render())
}

fn push_memory_query(
    bundle: &mut EvidenceBundle,
    source: &'static str,
    result: &Result<MemoryQuery>,
    max_rows: usize,
) {
    match result {
        Ok(query) => bundle.push(
            &query.receipt,
            dataframe_preview(&query.dataframe, max_rows),
        ),
        Err(error) => bundle.push_failure(source, &error.display_message()),
    }
}

#[component]
fn DeviceOverview(
    state: Option<Result<MemoryQuery>>,
    devices: Vec<DeviceMemory>,
    source_reported: bool,
) -> Element {
    match state {
        None => rsx! { LoadingPanel { label: "Loading device memory".to_string() } },
        Some(Err(error)) => rsx! { UnavailablePanel {
            label: "Device memory unavailable".to_string(),
            detail: error.display_message(),
        }},
        Some(Ok(_)) if devices.is_empty() => rsx! { UnavailablePanel {
            label: if source_reported { "No device memory samples".to_string() } else { "Device memory not reported".to_string() },
            detail: if source_reported {
                "The GPU collector returned no samples in the selected scope and window.".to_string()
            } else {
                "Enable the GPU collector to record physical-device memory usage.".to_string()
            },
        }},
        Some(Ok(query)) => {
            let current = devices.iter().map(|row| row.current_bytes).sum::<u64>();
            let peak = devices.iter().map(|row| row.peak_bytes).sum::<u64>();
            let capacity = devices.iter().map(|row| row.total_bytes).sum::<u64>();
            let headroom = capacity.saturating_sub(current);
            rsx! {
                div { class: "space-y-4",
                    if query.receipt.partial || query.receipt.failed_peers > 0 {
                        div { class: "rounded-md border border-amber-200 bg-amber-50 px-3 py-2 text-xs text-amber-900",
                            "Partial result · {query.receipt.failed_peers} peer endpoint(s) did not return device samples"
                        }
                    }
                    div { class: "grid grid-cols-5 divide-x divide-gray-200",
                        EvidenceMetric { label: "Devices", value: devices.len().to_string(), detail: None }
                        EvidenceMetric { label: "Current used", value: format_bytes(current), detail: Some(format_ratio(current, capacity)) }
                        EvidenceMetric { label: "Window peaks", value: format_bytes(peak), detail: Some("sum of per-device peaks".to_string()) }
                        EvidenceMetric { label: "Capacity", value: format_bytes(capacity), detail: None }
                        EvidenceMetric { label: "Current headroom", value: format_bytes(headroom), detail: Some(format_ratio(headroom, capacity)) }
                    }
                    DeviceMap { devices }
                }
            }
        }
    }
}

#[component]
fn DeviceMap(devices: Vec<DeviceMemory>) -> Element {
    let context = INVESTIGATION_CONTEXT.read().clone();
    let mut hosts = BTreeMap::<String, Vec<DeviceMemory>>::new();
    for device in devices {
        hosts.entry(device.host.clone()).or_default().push(device);
    }
    for rows in hosts.values_mut() {
        rows.sort_by_key(|row| (row.rank.unwrap_or(-1), row.device_id));
    }

    rsx! {
        div { class: "grid grid-cols-[repeat(auto-fill,minmax(180px,1fr))] gap-3 border-t border-gray-100 pt-3",
            for (host, rows) in hosts {
                div { class: "rounded-md border border-gray-200 bg-gray-50/60 p-2",
                    div { class: "mb-2 truncate text-xs font-medium text-gray-700", title: "{host}", "{host}" }
                    div { class: "grid grid-cols-4 gap-1.5",
                        for row in rows {
                            {
                                let active = context.device_id == Some(row.device_id)
                                    && (context.rank == row.rank || row.rank.is_none());
                                let ratio = percent(row.current_bytes, row.total_bytes);
                                let peak = percent(row.peak_bytes, row.total_bytes);
                                let host_for_click = row.host.clone();
                                let rank = row.rank;
                                let device_id = row.device_id;
                                rsx! {
                                    button {
                                        r#type: "button",
                                        class: if active {
                                            "min-h-14 rounded border border-blue-500 bg-blue-50 px-1 py-1 text-left ring-1 ring-blue-200"
                                        } else {
                                            "min-h-14 rounded border border-gray-200 bg-white px-1 py-1 text-left hover:border-blue-300 hover:bg-blue-50/40"
                                        },
                                        title: "GPU {device_id} · current {ratio:.1}% · window peak {peak:.1}% · {row.sample_count} samples",
                                        aria_pressed: active.to_string(),
                                        onclick: move |_| set_memory_device_context(rank, Some(&host_for_click), device_id),
                                        div { class: "font-mono text-xs font-semibold text-gray-900", "G{device_id}" }
                                        div { class: "text-xs tabular-nums text-gray-600", "C {ratio:.0}%" }
                                        div { class: "text-xs tabular-nums text-gray-500", "P {peak:.0}%" }
                                        div { class: "mt-1 h-1 rounded bg-gray-100",
                                            div { class: "h-full rounded bg-violet-500", style: "width: {ratio.clamp(0.0, 100.0):.1}%;" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn MemoryTimeline(
    state: Option<Result<MemoryQuery>>,
    selected: Option<DeviceMemory>,
    window_minutes: usize,
    source_reported: bool,
    mismatch_detail: Option<String>,
) -> Element {
    let Some(selected) = selected else {
        return rsx! { UnavailablePanel {
            label: if !source_reported {
                "Memory timeline not reported".to_string()
            } else if mismatch_detail.is_some() {
                "No matching device sample".to_string()
            } else {
                "Select a device".to_string()
            },
            detail: if !source_reported {
                "The GPU collector is not enabled, so no physical-device timeline is available.".to_string()
            } else {
                mismatch_detail.unwrap_or_else(|| "Choose a device in the physical map to inspect its sampled timeline.".to_string())
            },
        }};
    };
    match state {
        None => rsx! { LoadingPanel { label: "Loading memory timeline".to_string() } },
        Some(Err(error)) => rsx! { UnavailablePanel {
            label: "Memory timeline unavailable".to_string(),
            detail: error.display_message(),
        }},
        Some(Ok(query)) => {
            let mut points = parse_history(&query.dataframe)
                .into_iter()
                .filter(|point| {
                    point.device_id == selected.device_id && point.rank == selected.rank
                })
                .collect::<Vec<_>>();
            points.sort_by_key(|point| point.ts);
            if points.is_empty() {
                return rsx! { UnavailablePanel {
                    label: "No samples for the selected device".to_string(),
                    detail: format!("No GPU {} samples were returned inside the {window_minutes} minute window.", selected.device_id),
                }};
            }
            rsx! { MemoryLineChart { points, selected } }
        }
    }
}

#[component]
fn MemoryLineChart(points: Vec<MemoryPoint>, selected: DeviceMemory) -> Element {
    let mut fit_data = use_signal(|| true);
    let width = 900.0;
    let height = 146.0;
    let total = points
        .iter()
        .map(|point| point.total_bytes)
        .max()
        .unwrap_or(1)
        .max(1);
    let observed_min = points
        .iter()
        .map(|point| point.used_bytes)
        .min()
        .unwrap_or_default();
    let observed_max = points
        .iter()
        .map(|point| point.used_bytes)
        .max()
        .unwrap_or(total);
    let observed_span = observed_max.saturating_sub(observed_min);
    let fit_padding = observed_span
        .saturating_div(8)
        .max(observed_max.saturating_div(50))
        .max(256 * 1024 * 1024);
    let y_min = if fit_data() {
        observed_min.saturating_sub(fit_padding)
    } else {
        0
    };
    let y_max = if fit_data() {
        observed_max.saturating_add(fit_padding).min(total)
    } else {
        total
    };
    let y_span = y_max.saturating_sub(y_min).max(1);
    let start = points.first().map(|point| point.ts).unwrap_or_default();
    let end = points.last().map(|point| point.ts).unwrap_or(start);
    let time_span = (end - start).max(1) as f64;
    let line = points
        .iter()
        .map(|point| {
            let x = (point.ts - start) as f64 / time_span * width;
            let relative = point.used_bytes.saturating_sub(y_min) as f64 / y_span as f64;
            let y = (1.0 - relative) * height;
            format!("{x:.1},{y:.1}")
        })
        .collect::<Vec<_>>()
        .join(" ");
    let rank = selected
        .rank
        .map(|rank| format!("rank {rank} · "))
        .unwrap_or_default();
    let history_span = format_history_span(end.saturating_sub(start).max(0) as u64);
    let start_time = format_sample_time(start);
    let end_time = format_sample_time(end);
    let sample_age_us = unix_time_micros().saturating_sub(end.max(0) as u64);
    let sample_age = format_sample_age(sample_age_us);
    let sample_is_stale = sample_age_us > 15_000_000;
    rsx! {
        div {
            div { class: "mb-3 flex flex-wrap items-start justify-between gap-x-6 gap-y-1 text-xs",
                div { class: "min-w-0",
                    div { class: "font-semibold text-gray-900", "{rank}GPU {selected.device_id}" }
                    div { class: "max-w-xl truncate text-gray-500", title: "{selected.host}", "{selected.host}" }
                }
                div { class: "flex flex-wrap items-center gap-x-4 gap-y-1 text-gray-600",
                    span {
                        class: if sample_is_stale {
                            "rounded bg-amber-50 px-1.5 py-0.5 font-medium text-amber-800"
                        } else {
                            "rounded bg-emerald-50 px-1.5 py-0.5 font-medium text-emerald-700"
                        },
                        if sample_is_stale { "Stale · {sample_age}" } else { "Live · {sample_age}" }
                    }
                    span { "Latest " strong { class: "font-semibold text-gray-900", "{format_bytes(selected.current_bytes)}" } }
                    span { "Peak " strong { class: "font-semibold text-gray-900", "{format_bytes(selected.peak_bytes)}" } }
                    span { "Capacity " strong { class: "font-semibold text-gray-900", "{format_bytes(selected.total_bytes)}" } }
                    button {
                        r#type: "button",
                        class: "rounded border border-gray-200 bg-white px-2 py-1 font-medium text-gray-600 hover:border-gray-300 hover:bg-gray-50",
                        aria_pressed: fit_data().to_string(),
                        onclick: move |_| fit_data.toggle(),
                        if fit_data() { "Full capacity" } else { "Fit data" }
                    }
                }
            }
            div {
                class: "relative h-44",
                role: "img",
                aria_label: "{rank}GPU {selected.device_id} memory used over time; vertical scale from {format_bytes(y_min)} to {format_bytes(y_max)}",
                div { class: "absolute bottom-6 left-0 top-0 w-14",
                    for tick in 0..=4 {
                        {
                            let ratio = tick as f64 / 4.0;
                            let value = y_max.saturating_sub((ratio * y_span as f64) as u64);
                            let position = if tick == 0 || tick == 4 {
                                String::new()
                            } else {
                                format!("top: {:.2}%;", ratio * 100.0)
                            };
                            rsx! {
                                span {
                                    class: if tick == 0 {
                                        "absolute left-0 top-0 w-14 whitespace-nowrap text-right text-xs text-gray-500"
                                    } else if tick == 4 {
                                        "absolute bottom-0 left-0 w-14 whitespace-nowrap text-right text-xs text-gray-500"
                                    } else {
                                        "absolute left-0 w-14 -translate-y-1/2 whitespace-nowrap text-right text-xs text-gray-500"
                                    },
                                    style: "{position}",
                                    "{format_bytes(value)}"
                                }
                            }
                        }
                    }
                }
                div { class: "absolute bottom-6 left-16 right-3 top-0",
                    svg { class: "h-full w-full", view_box: "0 0 {width} {height}", preserve_aspect_ratio: "none",
                        for tick in 0..=4 {
                            {
                                let y = tick as f64 / 4.0 * height;
                                rsx! {
                                    line { x1: "0", y1: "{y}", x2: "{width}", y2: "{y}", stroke: "#e5e7eb", stroke_width: "1", vector_effect: "non-scaling-stroke" }
                                }
                            }
                        }
                        polyline { points: "{line}", fill: "none", stroke: "#7c3aed", stroke_width: "2.5", stroke_linejoin: "round", stroke_linecap: "round", vector_effect: "non-scaling-stroke" }
                        for point in points.iter() {
                            {
                                let x = (point.ts - start) as f64 / time_span * width;
                                let relative = point.used_bytes.saturating_sub(y_min) as f64 / y_span as f64;
                                let y = (1.0 - relative) * height;
                                let time = format_sample_time(point.ts);
                                let used = format_bytes(point.used_bytes);
                                rsx! {
                                    circle {
                                        cx: "{x}", cy: "{y}", r: "5",
                                        fill: "transparent", stroke: "transparent",
                                        class: "cursor-crosshair",
                                        title { "{time} · {used}" }
                                    }
                                }
                            }
                        }
                    }
                }
                span { class: "absolute bottom-0 left-16 text-xs text-gray-500", "{start_time} · -{history_span}" }
                span { class: "absolute bottom-0 right-3 text-xs text-gray-500", "{end_time} · Latest" }
            }
        }
    }
}

fn format_history_span(duration_us: u64) -> String {
    let seconds = duration_us / 1_000_000;
    if seconds >= 3600 {
        format!("{:.1}h", seconds as f64 / 3600.0)
    } else if seconds >= 60 {
        format!("{:.1}m", seconds as f64 / 60.0)
    } else {
        format!("{seconds}s")
    }
}

fn format_sample_age(age_us: u64) -> String {
    let seconds = age_us / 1_000_000;
    if seconds < 2 {
        "now".to_string()
    } else if seconds < 60 {
        format!("{seconds}s ago")
    } else {
        format!("{:.1}m ago", seconds as f64 / 60.0)
    }
}

fn format_sample_time(timestamp_us: i64) -> String {
    chrono::DateTime::from_timestamp_micros(timestamp_us)
        .map(|timestamp| timestamp.format("%H:%M:%S UTC").to_string())
        .unwrap_or_else(|| "unknown time".to_string())
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
        .try_into()
        .unwrap_or(u64::MAX)
}

#[component]
fn EvidenceDataFrame(
    state: Option<Result<MemoryQuery>>,
    loading: &'static str,
    empty: &'static str,
) -> Element {
    match state {
        None => rsx! { div { class: "p-4", LoadingPanel { label: loading.to_string() } } },
        Some(Err(error)) => {
            rsx! { div { class: "p-4", UnavailablePanel { label: format!("{empty} available"), detail: error.display_message() } } }
        }
        Some(Ok(query)) if dataframe_rows(&query.dataframe) == 0 => {
            rsx! { div { class: "p-4", UnavailablePanel {
                label: empty.to_string(),
                detail: "The selected scope returned zero rows; no conclusion is inferred.".to_string(),
            } } }
        }
        Some(Ok(query)) => rsx! {
            if query.receipt.partial || query.receipt.failed_peers > 0 {
                div { class: "border-b border-amber-200 bg-amber-50 px-4 py-2 text-xs text-amber-900", "Partial result · {query.receipt.failed_peers} failed peer endpoint(s)" }
            }
            DataFrameView { df: query.dataframe }
        },
    }
}

async fn memory_query(
    sql: &str,
    source: &'static str,
    request: &EvidenceRequest,
) -> Result<MemoryQuery> {
    let client = ApiClient::new();
    if request.scope == EvidenceScope::ClusterFanout {
        client.cluster_query(sql, true).await.map(|response| {
            let rows = dataframe_rows(&response.dataframe);
            MemoryQuery {
                receipt: EvidenceReceipt::cluster(
                    source,
                    request,
                    rows,
                    response.meta.nodes_queried,
                    response.meta.nodes_failed.len(),
                    response.meta.partial,
                ),
                dataframe: response.dataframe,
            }
        })
    } else {
        client.execute_query(sql).await.map(|dataframe| {
            let rows = dataframe_rows(&dataframe);
            MemoryQuery {
                receipt: EvidenceReceipt::local(source, request, rows),
                dataframe,
            }
        })
    }
}

fn device_summary_sql(window_us: u64) -> String {
    format!("WITH samples AS ( \
      SELECT CAST(COALESCE((SELECT value FROM process.envs WHERE name = 'RANK' LIMIT 1), '-1') AS INT) AS rank, \
        CAST(COALESCE((SELECT value FROM process.envs WHERE name = 'LOCAL_RANK' LIMIT 1), '-1') AS INT) AS local_rank, \
        device_id, used_bytes, total_bytes, ts, \
        MAX(used_bytes) OVER (PARTITION BY device_id) AS peak_used_bytes, \
        COUNT(*) OVER (PARTITION BY device_id) AS sample_count, \
        ROW_NUMBER() OVER (PARTITION BY device_id ORDER BY ts DESC) AS recency \
      FROM gpu.utilization WHERE ts >= GREATEST(COALESCE((SELECT MAX(ts) FROM gpu.utilization), 0) - {window_us}, 0) \
    ) SELECT rank, device_id, used_bytes AS current_used_bytes, peak_used_bytes, total_bytes, sample_count \
      FROM samples WHERE recency = 1 AND (local_rank < 0 OR device_id = local_rank) \
      ORDER BY rank, device_id LIMIT 16")
}

fn device_history_sql(window_us: u64) -> String {
    format!("SELECT CAST(COALESCE((SELECT value FROM process.envs WHERE name = 'RANK' LIMIT 1), '-1') AS INT) AS rank, \
      device_id, ts, used_bytes, total_bytes FROM gpu.utilization \
      WHERE ts >= GREATEST(COALESCE((SELECT MAX(ts) FROM gpu.utilization), 0) - {window_us}, 0) \
        AND (COALESCE((SELECT value FROM process.envs WHERE name = 'LOCAL_RANK' LIMIT 1), '-1') = '-1' \
          OR device_id = CAST((SELECT value FROM process.envs WHERE name = 'LOCAL_RANK' LIMIT 1) AS INT)) \
      ORDER BY ts ASC LIMIT 1920")
}

const ALLOCATOR_SQL: &str = "SELECT rank, local_step, round(allocated, 1) AS allocated_mb, \
  round(max_allocated, 1) AS peak_allocated_mb, round(cached, 1) AS reserved_mb, \
  round(CASE WHEN cached > 0 THEN allocated * 100.0 / cached ELSE 0 END, 1) AS allocated_of_reserved_pct \
  FROM python.torch_trace WHERE rank >= 0 AND allocated >= 0 AND stage LIKE 'post %' \
  ORDER BY local_step DESC, seq DESC LIMIT 1";

const ALLOCATION_EVIDENCE_SQL: &str = "SELECT rank, module, stage, count(*) AS samples, \
  round(avg(allocated_delta), 1) AS avg_alloc_delta_mb, \
  round(max(allocated_delta), 1) AS max_alloc_delta_mb, \
  round(max(max_allocated_delta), 1) AS peak_growth_mb \
  FROM python.torch_trace WHERE stage LIKE 'post %' AND allocated_delta > 0 \
    AND module IS NOT NULL AND module != '' AND module != 'None' \
    AND local_step >= GREATEST(COALESCE((SELECT max(local_step) FROM python.torch_trace), 0) - 9, 1) \
  GROUP BY rank, module, stage ORDER BY peak_growth_mb DESC LIMIT 20";

fn parse_devices(dataframe: &DataFrame, nodes: &[Node]) -> Vec<DeviceMemory> {
    let rank_col = column(dataframe, "rank");
    let device_col = column(dataframe, "device_id");
    let current_col = column(dataframe, "current_used_bytes");
    let peak_col = column(dataframe, "peak_used_bytes");
    let total_col = column(dataframe, "total_bytes");
    let samples_col = column(dataframe, "sample_count");
    let mut result = Vec::new();
    for row in dataframe.iter() {
        let raw_rank = ele_i64(row.get(rank_col.unwrap_or(usize::MAX)))
            .and_then(|value| i32::try_from(value).ok());
        let rank = raw_rank.filter(|value| *value >= 0);
        let Some(device_id) = ele_i64(row.get(device_col.unwrap_or(usize::MAX)))
            .and_then(|value| i32::try_from(value).ok())
        else {
            continue;
        };
        let current_bytes = ele_u64(row.get(current_col.unwrap_or(usize::MAX))).unwrap_or_default();
        let peak_bytes = ele_u64(row.get(peak_col.unwrap_or(usize::MAX))).unwrap_or(current_bytes);
        let total_bytes = ele_u64(row.get(total_col.unwrap_or(usize::MAX))).unwrap_or_default();
        let sample_count =
            ele_u64(row.get(samples_col.unwrap_or(usize::MAX))).unwrap_or_default() as usize;
        let host = rank
            .and_then(|rank| {
                nodes
                    .iter()
                    .find(|node| node.rank == Some(rank))
                    .map(|node| node.host.clone())
            })
            .unwrap_or_else(|| {
                rank.map(|rank| format!("rank {rank}"))
                    .unwrap_or_else(|| "local node".to_string())
            });
        result.push(DeviceMemory {
            rank,
            device_id,
            host,
            current_bytes,
            peak_bytes,
            total_bytes,
            sample_count,
        });
    }
    result.sort_by(|left, right| {
        left.host
            .cmp(&right.host)
            .then(left.rank.cmp(&right.rank))
            .then(left.device_id.cmp(&right.device_id))
    });
    result
}

fn parse_history(dataframe: &DataFrame) -> Vec<MemoryPoint> {
    let rank_col = column(dataframe, "rank");
    let device_col = column(dataframe, "device_id");
    let ts_col = column(dataframe, "ts");
    let used_col = column(dataframe, "used_bytes");
    let total_col = column(dataframe, "total_bytes");
    dataframe
        .iter()
        .filter_map(|row| {
            let rank = ele_i64(row.get(rank_col.unwrap_or(usize::MAX)))
                .and_then(|value| i32::try_from(value).ok())
                .filter(|value| *value >= 0);
            Some(MemoryPoint {
                rank,
                device_id: i32::try_from(ele_i64(row.get(device_col.unwrap_or(usize::MAX)))?)
                    .ok()?,
                ts: ele_i64(row.get(ts_col.unwrap_or(usize::MAX)))?,
                used_bytes: ele_u64(row.get(used_col.unwrap_or(usize::MAX)))?,
                total_bytes: ele_u64(row.get(total_col.unwrap_or(usize::MAX)))?,
            })
        })
        .collect()
}

fn memory_context_requested(context: &InvestigationContext) -> bool {
    context.rank.is_some() || context.host.is_some() || context.device_id.is_some()
}

fn memory_context_match(
    request: &EvidenceRequest,
    context: &InvestigationContext,
    matched: bool,
) -> ContextMatch {
    if !memory_context_requested(context) {
        ContextMatch::Unpinned
    } else {
        request.context_match(matched)
    }
}

fn select_device_for_context(
    devices: &[DeviceMemory],
    context: &InvestigationContext,
) -> Option<DeviceMemory> {
    if !memory_context_requested(context) {
        return devices.first().cloned();
    }

    devices
        .iter()
        .find(|device| {
            context.rank.is_none_or(|rank| device.rank == Some(rank))
                && context
                    .host
                    .as_ref()
                    .is_none_or(|host| device.host == *host)
                && context
                    .device_id
                    .is_none_or(|device_id| device.device_id == device_id)
        })
        .cloned()
}

fn memory_context_label(context: &InvestigationContext) -> String {
    let mut parts = Vec::new();
    if let Some(rank) = context.rank {
        parts.push(format!("rank {rank}"));
    }
    if let Some(host) = context.host.as_ref() {
        parts.push(host.clone());
    }
    if let Some(device_id) = context.device_id {
        parts.push(format!("GPU {device_id}"));
    }
    parts.join(" · ")
}

fn dataframe_rows(dataframe: &DataFrame) -> usize {
    dataframe
        .cols
        .iter()
        .map(|column| column.len())
        .max()
        .unwrap_or_default()
}

fn column(dataframe: &DataFrame, name: &str) -> Option<usize> {
    dataframe.names.iter().position(|candidate| {
        candidate.eq_ignore_ascii_case(name)
            || candidate
                .to_ascii_lowercase()
                .ends_with(&format!("_{name}"))
    })
}

fn ele_i64(value: Option<&Ele>) -> Option<i64> {
    match value? {
        Ele::I64(value) => Some(*value),
        Ele::I32(value) => Some(i64::from(*value)),
        Ele::F64(value) => Some(*value as i64),
        Ele::F32(value) => Some(*value as i64),
        Ele::Text(value) => value.parse().ok(),
        _ => None,
    }
}

fn ele_u64(value: Option<&Ele>) -> Option<u64> {
    ele_i64(value).and_then(|value| u64::try_from(value).ok())
}

fn percent(value: u64, total: u64) -> f64 {
    if total == 0 {
        0.0
    } else {
        value as f64 / total as f64 * 100.0
    }
}

fn format_ratio(value: u64, total: u64) -> String {
    if total == 0 {
        "—".to_string()
    } else {
        format!("{:.1}%", percent(value, total))
    }
}

fn format_bytes(bytes: u64) -> String {
    match bytes {
        value if value >= 1 << 30 => format!("{:.1} GiB", value as f64 / (1u64 << 30) as f64),
        value if value >= 1 << 20 => format!("{:.1} MiB", value as f64 / (1u64 << 20) as f64),
        value if value >= 1 << 10 => format!("{:.1} KiB", value as f64 / (1u64 << 10) as f64),
        value => format!("{value} B"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_queries_are_bounded_and_keep_scope_explicit() {
        let summary = device_summary_sql(300_000_000);
        let history = device_history_sql(300_000_000);
        assert!(summary.contains("LOCAL_RANK"));
        assert!(summary.contains("LIMIT 16"));
        assert!(history.contains("LIMIT 1920"));
        assert!(ALLOCATOR_SQL.contains("LIMIT 1"));
        assert!(ALLOCATION_EVIDENCE_SQL.contains("LIMIT 20"));
    }

    #[test]
    fn byte_and_ratio_labels_are_measurements() {
        assert_eq!(format_bytes(8 << 30), "8.0 GiB");
        assert_eq!(format_ratio(3, 4), "75.0%");
        assert_eq!(format_ratio(1, 0), "—");
        assert_eq!(format_history_span(45_000_000), "45s");
        assert_eq!(format_history_span(300_000_000), "5.0m");
        assert_eq!(format_sample_age(1_000_000), "now");
        assert_eq!(format_sample_age(12_000_000), "12s ago");
        assert_eq!(format_sample_time(0), "00:00:00 UTC");
    }

    #[test]
    fn pinned_memory_coordinates_never_fall_back_to_another_device() {
        let devices = vec![DeviceMemory {
            rank: Some(0),
            device_id: 0,
            host: "node-00".to_string(),
            current_bytes: 1,
            peak_bytes: 2,
            total_bytes: 4,
            sample_count: 1,
        }];
        let context = InvestigationContext {
            rank: Some(58),
            host: Some("node-07".to_string()),
            device_id: Some(2),
            ..Default::default()
        };

        assert_eq!(select_device_for_context(&devices, &context), None);
    }

    #[test]
    fn memory_defaults_to_first_device_only_without_pinned_coordinates() {
        let device = DeviceMemory {
            rank: Some(0),
            device_id: 0,
            host: "node-00".to_string(),
            current_bytes: 1,
            peak_bytes: 2,
            total_bytes: 4,
            sample_count: 1,
        };

        assert_eq!(
            select_device_for_context(
                std::slice::from_ref(&device),
                &InvestigationContext::default(),
            ),
            Some(device)
        );
    }
}
