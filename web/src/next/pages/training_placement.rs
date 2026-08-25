use std::collections::{BTreeMap, BTreeSet};

use dioxus::prelude::*;
use probing_proto::prelude::{DataFrame, Ele, Node};

use crate::state::investigation::{
    set_memory_device_context, set_training_rank_context, INVESTIGATION_CONTEXT,
};
use crate::utils::error::Result;

use super::super::components::EvidenceLink;
use super::super::model::{format_duration, StepHealth};
use super::super::routes::NextRoute;

#[derive(Clone, Debug, PartialEq, Eq)]
struct PlacementProcess {
    rank: Option<i32>,
    local_rank: Option<i32>,
    addr: String,
    role_label: Option<String>,
    coordinates: Vec<(String, String)>,
    status: Option<String>,
    timestamp: u64,
    layout: Option<RankLayoutSummary>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PlacementHost {
    name: String,
    processes: Vec<PlacementProcess>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct PlacementModel {
    hosts: Vec<PlacementHost>,
    observed_ranks: usize,
    expected_ranks: usize,
    has_parallel_coordinates: bool,
    parallel_sizes: Vec<(String, usize)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PlacementGroup {
    Focus,
    Tensor,
    Data,
    Pipeline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PlacementGroupSizes {
    tensor: usize,
    data: usize,
    pipeline: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct PlacementTooltipPosition {
    x: f64,
    y: f64,
}

#[derive(Clone, Debug, PartialEq)]
struct GroupCommunicationSample {
    rank: i32,
    op: String,
    group_size: usize,
    participants: Vec<i32>,
    calls: usize,
    avg_ms: f64,
    max_ms: f64,
    total_bytes: u64,
}

#[derive(Clone, Debug, PartialEq)]
struct GroupCommunicationSummary {
    calls: usize,
    avg_ms: f64,
    max_ms: f64,
    total_bytes: u64,
    sampled_ranks: usize,
    ops: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
enum GroupCommunicationEvidence {
    Loading,
    Unavailable(String),
    Loaded(Vec<GroupCommunicationSample>),
}

#[derive(Clone, Debug, PartialEq)]
struct RankMemorySample {
    rank: i32,
    local_step: i64,
    allocated_mb: f64,
    peak_allocated_mb: f64,
    reserved_mb: f64,
}

#[derive(Clone, Debug, PartialEq)]
enum RankMemoryEvidence {
    Loading,
    Unavailable(String),
    Loaded(Vec<RankMemorySample>),
}

#[derive(Clone, Debug, PartialEq)]
struct DeviceMemorySample {
    rank: Option<i32>,
    device_id: i32,
    current_used_bytes: u64,
    peak_used_bytes: u64,
    total_bytes: u64,
    sample_count: usize,
}

#[derive(Clone, Debug, PartialEq)]
enum DeviceMemoryEvidence {
    Loading,
    Unavailable(String),
    Loaded(Vec<DeviceMemorySample>),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct RankLayoutSummary {
    layer_start: Option<i32>,
    layer_end: Option<i32>,
    module_count: usize,
    model_chunks: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ModuleCatalogEntry {
    model_chunk: i32,
    module_path: String,
    module_type: String,
    depth: usize,
    local_layer_index: Option<i32>,
    global_layer_index: Option<i32>,
    parameter_count: u64,
    trainable_parameter_count: u64,
    dtype: String,
    device: String,
    is_leaf: bool,
}

#[component]
pub(super) fn TrainingPlacement(
    nodes: Vec<Node>,
    local_step: Option<i64>,
    step_health: Option<StepHealth>,
    group_communication: Option<Result<DataFrame>>,
    rank_memory: Option<Result<DataFrame>>,
    device_memory: Option<Result<DataFrame>>,
    megatron_rank_layout: Option<Result<DataFrame>>,
    megatron_module_catalog: Option<Result<DataFrame>>,
) -> Element {
    let layouts = rank_layout_evidence(megatron_rank_layout.as_ref());
    let placement = build_placement(&nodes, &layouts);
    rsx! {
        PlacementDiagram {
            placement,
            local_step,
            step_health,
            group_communication,
            rank_memory,
            device_memory,
            megatron_module_catalog,
        }
    }
}

#[component]
fn PlacementDiagram(
    placement: PlacementModel,
    local_step: Option<i64>,
    step_health: Option<StepHealth>,
    group_communication: Option<Result<DataFrame>>,
    rank_memory: Option<Result<DataFrame>>,
    device_memory: Option<Result<DataFrame>>,
    megatron_module_catalog: Option<Result<DataFrame>>,
) -> Element {
    let missing_ranks = placement
        .expected_ranks
        .saturating_sub(placement.observed_ranks);
    rsx! {
        div { class: "space-y-3",
            div { class: "flex flex-wrap items-center gap-x-4 gap-y-2 text-xs text-gray-600",
                span { class: "font-medium text-gray-900", "{placement.hosts.len()} hosts" }
                span { "{placement.observed_ranks} / {placement.expected_ranks} ranks" }
                if placement.has_parallel_coordinates {
                    for (dimension, size) in placement.parallel_sizes.iter() {
                        span { class: "font-mono font-semibold uppercase text-violet-700",
                            "{dimension}{size}"
                        }
                    }
                } else if placement.expected_ranks > 1 {
                    span { class: "text-gray-500", "parallel coordinates not reported" }
                }
                if missing_ranks > 0 {
                    span { class: "rounded-full bg-amber-100 px-2 py-0.5 font-medium text-amber-800",
                        "{missing_ranks} missing"
                    }
                }
            }
            LogicalTopology { placement: placement.clone(), local_step }
            PlacementOverview { placement, local_step, step_health, group_communication, rank_memory, device_memory, megatron_module_catalog }
        }
    }
}

#[component]
fn LogicalTopology(placement: PlacementModel, local_step: Option<i64>) -> Element {
    let mut selected_dp = use_signal(|| 0);
    let processes = placement
        .hosts
        .iter()
        .flat_map(|host| host.processes.iter())
        .cloned()
        .collect::<Vec<_>>();
    let dp_values = processes
        .iter()
        .filter_map(|process| coordinate_i32(process, "dp"))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if dp_values.is_empty()
        || !processes.iter().any(|process| {
            coordinate(process, "pp").is_some() && coordinate(process, "tp").is_some()
        })
    {
        return rsx! {};
    }
    let pinned_dp = INVESTIGATION_CONTEXT
        .read()
        .rank
        .and_then(|rank| processes.iter().find(|process| process.rank == Some(rank)))
        .and_then(|process| coordinate_i32(process, "dp"));
    let default_dp = pinned_dp.unwrap_or(dp_values[0]);
    let requested_dp = selected_dp();
    let selected = if dp_values.contains(&requested_dp) {
        requested_dp
    } else {
        default_dp
    };
    let pp_values = processes
        .iter()
        .filter(|process| coordinate_i32(process, "dp") == Some(selected))
        .filter_map(|process| coordinate_i32(process, "pp"))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let tp_values = processes
        .iter()
        .filter(|process| coordinate_i32(process, "dp") == Some(selected))
        .filter_map(|process| coordinate_i32(process, "tp"))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let pp_count = pp_values.len();
    let mut cells = Vec::new();
    for tp in &tp_values {
        for pp in &pp_values {
            let matching = processes
                .iter()
                .filter(|process| {
                    coordinate_i32(process, "dp") == Some(selected)
                        && coordinate_i32(process, "tp") == Some(*tp)
                        && coordinate_i32(process, "pp") == Some(*pp)
                })
                .cloned()
                .collect::<Vec<_>>();
            cells.push((*tp, *pp, matching));
        }
    }

    rsx! {
        div { class: "rounded-md border border-gray-200 bg-white p-3",
            div { class: "mb-3 flex flex-wrap items-center justify-between gap-2",
                div {
                    div { class: "text-xs font-semibold uppercase tracking-wide text-gray-600", "Logical model placement" }
                    div { class: "mt-0.5 text-xs text-gray-500", "Pipeline stages by tensor-parallel lane; choose a data-parallel replica." }
                }
                div { class: "flex flex-wrap items-center gap-1",
                    for dp in dp_values {
                        button {
                            r#type: "button",
                            class: if dp == selected { "rounded border border-blue-600 bg-blue-600 px-2 py-1 font-mono text-xs font-semibold text-white" } else { "rounded border border-gray-300 bg-white px-2 py-1 font-mono text-xs text-gray-700 hover:border-blue-400" },
                            onclick: move |_| selected_dp.set(dp),
                            "DP{dp}"
                        }
                    }
                }
            }
            div { class: "overflow-x-auto",
                div {
                    class: "grid min-w-max gap-1.5",
                    style: "grid-template-columns: 3rem repeat({pp_count}, minmax(7.5rem, 1fr));",
                    span {}
                    for pp in &pp_values {
                        div { class: "rounded bg-amber-50 px-2 py-1 text-center font-mono text-xs font-semibold text-amber-900", "PP{pp}" }
                    }
                    for tp in &tp_values {
                        div { class: "flex items-center justify-center rounded bg-violet-50 px-2 font-mono text-xs font-semibold text-violet-900", "TP{tp}" }
                        for (_, pp, processes) in cells.iter().filter(|(cell_tp, _, _)| cell_tp == tp) {
                            if !processes.is_empty() {
                                div { class: "grid gap-1 rounded border border-gray-200 bg-white p-1",
                                    for process in processes {
                                        LogicalRankCell { process: process.clone(), pp: *pp, local_step }
                                    }
                                }
                            } else {
                                div { class: "flex min-h-12 items-center justify-center rounded border border-dashed border-gray-200 text-xs text-gray-400", "missing" }
                            }
                        }
                    }
                }
            }
            div { class: "mt-2 text-xs text-gray-500", "Click a rank to inspect its static module tree." }
        }
    }
}

#[component]
fn LogicalRankCell(process: PlacementProcess, pp: i32, local_step: Option<i64>) -> Element {
    let rank = process.rank;
    let rank_label = rank
        .map(|rank| format!("R{rank}"))
        .unwrap_or_else(|| "R?".to_string());
    let layer_label = process
        .layout
        .as_ref()
        .map(format_rank_layout)
        .unwrap_or_else(|| "layers not reported".to_string());
    let title = format!("{rank_label} · PP{pp} · {layer_label}");
    rsx! {
        button {
            r#type: "button",
            class: "min-h-12 rounded border border-gray-300 bg-gray-50 px-2 py-1 text-left hover:border-blue-500 hover:bg-blue-50 focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-700",
            title: "{title}",
            onclick: move |_| {
                if let Some(rank) = rank {
                    set_training_rank_context(rank, local_step, None);
                }
            },
            div { class: "font-mono text-xs font-semibold text-gray-950", "{rank_label}" }
            div { class: "mt-0.5 truncate font-mono text-xs text-gray-600", "{layer_label}" }
        }
    }
}

#[component]
fn PlacementOverview(
    placement: PlacementModel,
    local_step: Option<i64>,
    step_health: Option<StepHealth>,
    group_communication: Option<Result<DataFrame>>,
    rank_memory: Option<Result<DataFrame>>,
    device_memory: Option<Result<DataFrame>>,
    megatron_module_catalog: Option<Result<DataFrame>>,
) -> Element {
    let mut hovered_rank = use_signal(|| None::<i32>);
    let mut tooltip_rank = use_signal(|| None::<i32>);
    let tooltip_position = use_signal(PlacementTooltipPosition::default);
    let pinned_rank = INVESTIGATION_CONTEXT.read().rank;
    let active_rank = hovered_rank().or(pinned_rank);
    let active_is_pinned = placement_selection_is_pinned(active_rank, pinned_rank);
    let active_selection = active_rank.and_then(|rank| {
        placement.hosts.iter().find_map(|host| {
            host.processes
                .iter()
                .find(|process| process.rank == Some(rank))
                .map(|process| (host.name.clone(), process.clone()))
        })
    });
    let active_process = active_selection
        .as_ref()
        .map(|(_, process)| process.clone());
    let group_sizes = active_process
        .as_ref()
        .and_then(|active| placement_group_sizes(&placement, active));
    let placement_memory = device_memory_evidence(device_memory.as_ref());
    let has_memory_heat = matches!(
        &placement_memory,
        DeviceMemoryEvidence::Loaded(samples) if !samples.is_empty()
    );
    let tooltip_selection = tooltip_rank().and_then(|rank| {
        placement.hosts.iter().find_map(|host| {
            host.processes
                .iter()
                .find(|process| process.rank == Some(rank))
                .map(|process| (host.name.clone(), process.clone()))
        })
    });
    let tooltip_style = placement_tooltip_style(tooltip_position());

    rsx! {
        div {
            class: "rounded-md border border-gray-200 bg-gray-50 px-3 py-2.5",
            onmouseleave: move |_| hovered_rank.set(None),
            div { class: "mb-2 flex flex-wrap items-center justify-between gap-2",
                div { class: "flex items-center gap-2",
                    span { class: "text-xs font-medium uppercase tracking-wide text-gray-500", "Overview" }
                    if let Some(rank) = active_rank {
                        span { class: "font-mono text-xs font-semibold text-blue-700", "R{rank}" }
                        if active_is_pinned {
                            span { class: "text-xs text-blue-600", "pinned" }
                        }
                    }
                }
                div { class: "flex items-center gap-3 text-xs text-gray-500",
                    MemoryLegend { available: has_memory_heat }
                    GroupLegend { label: "TP", count: group_sizes.map(|sizes| sizes.tensor), class: "border-violet-500 bg-violet-100" }
                    GroupLegend { label: "DP", count: group_sizes.map(|sizes| sizes.data), class: "border-emerald-500 bg-emerald-100" }
                    GroupLegend { label: "PP", count: group_sizes.map(|sizes| sizes.pipeline), class: "border-amber-500 bg-amber-100" }
                    span { class: "text-gray-600", "Focus or hover to preview · click to pin" }
                }
            }
            div { class: "min-w-0",
                div { class: "overflow-x-auto pb-0.5",
                    div {
                        class: "flex min-w-max flex-wrap gap-1.5",
                        for (host_index, host) in placement.hosts.iter().enumerate() {
                            div {
                                class: "min-w-0 rounded border border-gray-200 bg-white p-1",
                                aria_label: "Host {host_index}: {host.name}",
                                div { class: "mb-1 truncate text-center font-mono text-xs text-gray-600", "H{host_index}" }
                                div {
                                    class: "grid justify-items-center gap-0.5",
                                    style: "grid-template-columns: repeat({host.processes.len().clamp(1, 8)}, 1.5rem);",
                                    for process in host.processes.iter() {
                                        PlacementCell {
                                            host: host.name.clone(),
                                            process: process.clone(),
                                            active: active_process.clone(),
                                            memory: placement_device_memory(&placement_memory, process),
                                            hovered_rank,
                                            tooltip_rank,
                                            tooltip_position,
                                            local_step,
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                div { class: "mt-2 flex w-full flex-wrap gap-x-3 gap-y-1 border-t border-gray-200 pt-2 text-xs text-gray-600",
                    for (host_index, host) in placement.hosts.iter().enumerate() {
                        span { class: "font-mono", "H{host_index} {host.name}" }
                    }
                }
            }
            if let Some((host, process)) = tooltip_selection {
                div {
                    class: "fixed inset-0 z-40 bg-transparent",
                    role: "presentation",
                    onclick: move |_| tooltip_rank.set(None),
                }
                div {
                    class: "fixed z-50 overflow-y-auto rounded-lg border border-gray-300 bg-white p-3 shadow-2xl",
                    style: "{tooltip_style}",
                    role: "dialog",
                    aria_label: "Selected rank evidence",
                    onclick: move |event| event.stop_propagation(),
                    button {
                        r#type: "button",
                        class: "absolute right-2 top-2 z-10 flex h-7 w-7 items-center justify-center rounded-md border border-gray-200 bg-white text-base text-gray-500 shadow-sm hover:bg-gray-50 hover:text-gray-900 focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-700",
                        aria_label: "Close selected rank evidence",
                        title: "Close",
                        onclick: move |_| tooltip_rank.set(None),
                        "×"
                    }
                    PlacementSelectionDetail {
                        host: host.clone(),
                        process: process.clone(),
                        group_sizes: placement_group_sizes(&placement, &process),
                        pinned: true,
                    }
                    PlacementLinkedEvidence {
                        placement: placement.clone(),
                        host,
                        process,
                        step_health: step_health.clone(),
                        group_communication: group_communication.clone(),
                        rank_memory: rank_memory.clone(),
                        device_memory: device_memory.clone(),
                        megatron_module_catalog: megatron_module_catalog.clone(),
                    }
                }
            }
        }
    }
}

fn placement_tooltip_style(position: PlacementTooltipPosition) -> String {
    let _ = position;
    "left: 50%; top: 50%; transform: translate(-50%, -50%); \
     width: min(80rem, calc(100vw - 2rem)); height: calc(100vh - 2rem);"
        .to_string()
}

fn placement_selection_is_pinned(active_rank: Option<i32>, pinned_rank: Option<i32>) -> bool {
    active_rank.is_some() && active_rank == pinned_rank
}

#[component]
fn PlacementSelectionDetail(
    host: String,
    process: PlacementProcess,
    group_sizes: Option<PlacementGroupSizes>,
    pinned: bool,
) -> Element {
    let layout = process.layout.as_ref().map(format_rank_layout);
    let rank = process
        .rank
        .map(|value| format!("rank {value}"))
        .unwrap_or_else(|| "rank unknown".to_string());
    let local_rank = process
        .local_rank
        .map(|value| format!("GPU {value}"))
        .unwrap_or_else(|| "GPU unknown".to_string());
    let status = process
        .status
        .unwrap_or_else(|| "unknown status".to_string());
    let role = process
        .role_label
        .unwrap_or_else(|| "role unknown".to_string());
    let coordinates = process
        .coordinates
        .iter()
        .map(|(dimension, value)| format!("{dimension}={value}"))
        .collect::<Vec<_>>()
        .join(" · ");

    rsx! {
        div {
            class: "mb-2 flex flex-wrap items-center gap-x-3 gap-y-1 rounded-md border border-blue-200 bg-blue-50/40 py-2 pl-3 pr-10 text-xs text-gray-700",
            aria_live: "polite",
            span { class: "font-semibold text-gray-950", "{rank}" }
            span { "{host}" }
            span { "{local_rank}" }
            span { "{status}" }
            span { class: "font-mono", "{role}" }
            if !coordinates.is_empty() {
                span { class: "font-mono", "{coordinates}" }
            }
            if let Some(layout) = layout {
                span { class: "font-mono font-medium text-cyan-800", "{layout}" }
            }
            if let Some(sizes) = group_sizes {
                span { class: "font-medium text-violet-800", "TP group {sizes.tensor}" }
                span { class: "font-medium text-emerald-800", "DP group {sizes.data}" }
                span { class: "font-medium text-amber-800", "PP group {sizes.pipeline}" }
            }
            span { class: "ml-auto font-medium text-blue-700", if pinned { "Pinned" } else { "Preview" } }
        }
    }
}

#[component]
fn PlacementLinkedEvidence(
    placement: PlacementModel,
    host: String,
    process: PlacementProcess,
    step_health: Option<StepHealth>,
    group_communication: Option<Result<DataFrame>>,
    rank_memory: Option<Result<DataFrame>>,
    device_memory: Option<Result<DataFrame>>,
    megatron_module_catalog: Option<Result<DataFrame>>,
) -> Element {
    let rank = process.rank;
    let status = process
        .status
        .clone()
        .filter(|status| !status.trim().is_empty())
        .unwrap_or_else(|| "not reported".to_string());
    let heartbeat = format_heartbeat_age(unix_time_micros(), process.timestamp);
    let endpoint = if process.addr.trim().is_empty() {
        "not reported".to_string()
    } else {
        process.addr.clone()
    };
    let rank_step = rank.and_then(|rank| {
        step_health.as_ref().and_then(|health| {
            health
                .rank_durations
                .iter()
                .find(|(candidate, _)| *candidate == rank)
                .map(|(_, duration)| (*duration, health.median_ms))
        })
    });
    let communication = group_communication_evidence(group_communication.as_ref());
    let memory = rank_memory_evidence(rank_memory.as_ref());
    let device_memory = device_memory_evidence(device_memory.as_ref());
    let has_parallel_groups = ["dp", "pp", "tp"]
        .iter()
        .all(|dimension| coordinate(&process, dimension).is_some());

    rsx! {
        div { class: "overflow-hidden rounded-md border border-gray-200 bg-white",
            div { class: "border-b border-gray-200 px-3 py-2",
                div { class: "text-xs font-semibold text-gray-900", "Selected rank evidence" }
                div { class: "mt-0.5 text-xs text-gray-500", "Reported state and measurements linked to the active placement rank." }
            }
            div { class: "grid divide-y divide-gray-100 sm:grid-cols-2 sm:divide-x sm:divide-y-0",
                div { class: "space-y-1 px-3 py-2 text-xs",
                    div { class: "font-medium text-gray-900", "Node" }
                    div { class: "flex flex-wrap gap-x-3 gap-y-1 text-gray-700",
                        span { "{host}" }
                        span { class: "font-mono", "{endpoint}" }
                    }
                    div { class: "flex flex-wrap gap-x-3 gap-y-1 text-gray-600",
                        span { "State · {status}" }
                        span { "Heartbeat · {heartbeat}" }
                    }
                }
                div { class: "space-y-1 px-3 py-2 text-xs",
                    div { class: "font-medium text-gray-900", "Latest rank step" }
                    if let Some((duration, median)) = rank_step {
                        div { class: "font-mono text-base font-semibold tabular-nums text-gray-950", "{format_duration(Some(duration))}" }
                        if let Some(median) = median.filter(|median| *median > 0.0) {
                            div { class: "text-gray-600", "{format_delta_percent(duration, median)} vs latest-rank median" }
                        }
                    } else {
                        div { class: "text-gray-500", "No comparable step sample for this rank." }
                    }
                }
            }
            RankModulesPanel { rank, state: megatron_module_catalog }
            RankMemoryPanel { rank, local_rank: process.local_rank, memory, device_memory }
            div { class: "border-t border-gray-200 px-3 py-2",
                div { class: "flex flex-wrap items-baseline justify-between gap-2",
                    div { class: "text-xs font-medium text-gray-900", "Communication groups" }
                    div { class: "text-xs text-gray-500", "Torch API wall time · exact participant-set match" }
                }
                if !has_parallel_groups {
                    div { class: "mt-2 text-xs text-gray-500", "TP / DP / PP coordinates were not reported for this rank." }
                } else {
                    div { class: "mt-2 divide-y divide-gray-100 border-y border-gray-100",
                        GroupEvidenceRow {
                            label: "TP",
                            tone: "text-violet-800",
                            members: placement_group_members(&placement, &process, PlacementGroup::Tensor),
                            communication: communication.clone(),
                        }
                        GroupEvidenceRow {
                            label: "DP",
                            tone: "text-emerald-800",
                            members: placement_group_members(&placement, &process, PlacementGroup::Data),
                            communication: communication.clone(),
                        }
                        GroupEvidenceRow {
                            label: "PP",
                            tone: "text-amber-800",
                            members: placement_group_members(&placement, &process, PlacementGroup::Pipeline),
                            communication,
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn RankModulesPanel(rank: Option<i32>, state: Option<Result<DataFrame>>) -> Element {
    let mut filter = use_signal(String::new);
    let mut layers_only = use_signal(|| false);
    let mut group_by_layer = use_signal(|| true);
    let all_entries = state
        .as_ref()
        .and_then(|result| result.as_ref().ok())
        .map(parse_module_catalog)
        .unwrap_or_default();
    let total_entries = all_entries.len();
    let query = filter().trim().to_ascii_lowercase();
    let entries = all_entries
        .into_iter()
        .filter(|entry| {
            (!layers_only()
                || entry.global_layer_index.is_some()
                || entry.local_layer_index.is_some())
                && module_matches_filter(entry, &query)
        })
        .collect::<Vec<_>>();
    let shown_entries = entries.len();
    let mut grouped_entries = BTreeMap::<Option<i32>, Vec<ModuleCatalogEntry>>::new();
    for entry in entries.iter().cloned() {
        grouped_entries
            .entry(entry.global_layer_index.or(entry.local_layer_index))
            .or_default()
            .push(entry);
    }
    rsx! {
        div { class: "border-t border-gray-200 px-3 py-2",
            div { class: "flex flex-wrap items-baseline justify-between gap-2",
                div { class: "text-xs font-medium text-gray-900", "Megatron modules and layers" }
                if let Some(rank) = rank {
                    div { class: "font-mono text-xs text-gray-500", "rank {rank} · {shown_entries} / {total_entries} modules" }
                }
            }
            if rank.is_some() && total_entries > 0 {
                div { class: "mt-2 flex flex-wrap items-center gap-2",
                    input {
                        r#type: "search",
                        class: "min-w-64 flex-1 rounded border border-gray-300 px-2 py-1.5 font-mono text-xs text-gray-700 placeholder:text-gray-400 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500",
                        placeholder: "Filter module, type, dtype, device, or L12",
                        value: "{filter}",
                        oninput: move |event| filter.set(event.value()),
                    }
                    button {
                        r#type: "button",
                        class: if layers_only() {
                            "rounded border border-cyan-300 bg-cyan-50 px-2 py-1.5 text-xs font-medium text-cyan-800"
                        } else {
                            "rounded border border-gray-300 bg-white px-2 py-1.5 text-xs font-medium text-gray-600 hover:bg-gray-50"
                        },
                        aria_pressed: layers_only().to_string(),
                        onclick: move |_| layers_only.toggle(),
                        "Layers only"
                    }
                    button {
                        r#type: "button",
                        class: if group_by_layer() {
                            "rounded border border-violet-300 bg-violet-50 px-2 py-1.5 text-xs font-medium text-violet-800"
                        } else {
                            "rounded border border-gray-300 bg-white px-2 py-1.5 text-xs font-medium text-gray-600 hover:bg-gray-50"
                        },
                        aria_pressed: group_by_layer().to_string(),
                        onclick: move |_| group_by_layer.toggle(),
                        "Group by layer"
                    }
                }
            }
            match state {
                None => rsx! { div { class: "mt-2 text-xs text-gray-500", "Loading static model layout…" } },
                Some(Err(error)) => rsx! { div { class: "mt-2 text-xs text-gray-500", "Static model layout unavailable · {error.display_message()}" } },
                Some(Ok(_)) if rank.is_none() => rsx! { div { class: "mt-2 text-xs text-gray-500", "Select a rank to load its static model layout." } },
                Some(Ok(_)) if total_entries == 0 => rsx! { div { class: "mt-2 text-xs text-gray-500", "This rank has not reported a static Megatron module catalog." } },
                Some(Ok(_)) if entries.is_empty() => rsx! { div { class: "mt-2 text-xs text-gray-500", "No modules match the current filters." } },
                Some(Ok(_)) => rsx! {
                    div {
                        class: "mt-2 overflow-auto rounded border border-gray-200",
                        style: "max-height: 60vh;",
                        if group_by_layer() {
                            for (layer, group) in grouped_entries {
                                ModuleLayerGroup {
                                    layer,
                                    entries: group,
                                    expanded: !query.is_empty(),
                                }
                            }
                        } else {
                            for entry in entries {
                                ModuleCatalogRow { entry }
                            }
                        }
                    }
                },
            }
        }
    }
}

#[component]
fn ModuleLayerGroup(
    layer: Option<i32>,
    entries: Vec<ModuleCatalogEntry>,
    expanded: bool,
) -> Element {
    let label = layer
        .map(|index| format!("Layer {index}"))
        .unwrap_or_else(|| "Shared / non-layer modules".to_string());
    let parameters = entries
        .iter()
        .map(|entry| entry.parameter_count)
        .sum::<u64>();
    rsx! {
        details { class: "border-b border-gray-200 last:border-b-0", open: expanded,
            summary { class: "flex cursor-pointer list-none items-center justify-between gap-3 bg-gray-50 px-2 py-2 text-xs hover:bg-gray-100",
                span { class: "font-semibold text-gray-800", "{label}" }
                span { class: "font-mono text-gray-500", "{entries.len()} modules · {format_parameter_count(parameters)} params" }
            }
            div { class: "border-t border-gray-100",
                for entry in entries {
                    ModuleCatalogRow { entry }
                }
            }
        }
    }
}

fn module_matches_filter(entry: &ModuleCatalogEntry, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let layer = entry
        .global_layer_index
        .or(entry.local_layer_index)
        .map(|index| format!("l{index}"))
        .unwrap_or_default();
    [
        entry.module_path.as_str(),
        entry.module_type.as_str(),
        entry.dtype.as_str(),
        entry.device.as_str(),
        layer.as_str(),
    ]
    .iter()
    .any(|value| value.to_ascii_lowercase().contains(query))
}

#[component]
fn ModuleCatalogRow(entry: ModuleCatalogEntry) -> Element {
    let indent = entry.depth.min(8) as f32 * 0.75;
    let module_type = entry
        .module_type
        .rsplit('.')
        .next()
        .unwrap_or(entry.module_type.as_str())
        .to_string();
    let layer = entry
        .global_layer_index
        .map(|index| format!("L{index}"))
        .or_else(|| {
            entry
                .local_layer_index
                .map(|index| format!("local L{index}"))
        });
    let parameters = format_parameter_count(entry.parameter_count);
    let trainable = format_parameter_count(entry.trainable_parameter_count);
    let placement = [entry.dtype.as_str(), entry.device.as_str()]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
    rsx! {
        div {
            class: "grid grid-cols-[minmax(16rem,1fr)_auto] gap-3 border-b border-gray-100 px-2 py-1.5 text-xs last:border-b-0",
            div { class: "min-w-0", style: "padding-left: {indent}rem;",
                div { class: if entry.is_leaf { "truncate font-mono font-medium text-gray-900" } else { "truncate font-mono text-gray-700" }, "{entry.module_path}" }
                div { class: "truncate text-gray-500", "chunk {entry.model_chunk} · {module_type}" }
            }
            div { class: "text-right",
                if let Some(layer) = layer {
                    div { class: "font-mono font-semibold text-cyan-800", "{layer}" }
                }
                div { class: "font-mono text-gray-600", "{parameters} params · {trainable} trainable" }
                if !placement.is_empty() {
                    div { class: "font-mono text-gray-500", "{placement}" }
                }
            }
        }
    }
}

#[component]
fn RankMemoryPanel(
    rank: Option<i32>,
    local_rank: Option<i32>,
    memory: RankMemoryEvidence,
    device_memory: DeviceMemoryEvidence,
) -> Element {
    let sample = match &memory {
        RankMemoryEvidence::Loaded(samples) => {
            rank.and_then(|rank| samples.iter().find(|sample| sample.rank == rank).cloned())
        }
        RankMemoryEvidence::Loading | RankMemoryEvidence::Unavailable(_) => None,
    };
    let device_sample =
        select_device_memory_sample(&device_memory, rank, local_rank, sample.is_some());
    rsx! {
        div { class: "border-t border-gray-200 px-3 py-2",
            div { class: "flex flex-wrap items-baseline justify-between gap-2",
                div { class: "text-xs font-medium text-gray-900", "GPU memory" }
                div { class: "flex items-center gap-3 text-xs",
                    span { class: "text-gray-500", "Device sampling and PyTorch allocator use distinct scopes" }
                    EvidenceLink {
                        route: NextRoute::Memory {},
                        label: "Open Memory →".to_string(),
                        class_name: "font-medium text-blue-600 hover:underline".to_string(),
                    }
                }
            }
            DeviceMemoryRow { sample: device_sample, evidence: device_memory }
            if let Some(sample) = sample {
                div { class: "mt-2 grid grid-cols-2 gap-x-3 gap-y-2 border-t border-gray-100 pt-2 text-xs sm:grid-cols-4",
                    MemoryMetric { label: "Allocated", value: format_memory_mb(sample.allocated_mb), detail: Some(format!("step {}", sample.local_step)) }
                    MemoryMetric { label: "Peak allocated", value: format_memory_mb(sample.peak_allocated_mb), detail: Some("since allocator reset".to_string()) }
                    MemoryMetric { label: "Reserved", value: format_memory_mb(sample.reserved_mb), detail: Some(format!("{} gap", format_memory_mb((sample.reserved_mb - sample.allocated_mb).max(0.0)))) }
                    MemoryMetric { label: "Allocated / reserved", value: format_memory_ratio(sample.allocated_mb, sample.reserved_mb), detail: Some(format!("{} below peak", format_memory_mb((sample.peak_allocated_mb - sample.allocated_mb).max(0.0)))) }
                }
            } else {
                div { class: "mt-2 text-xs text-gray-500",
                    match memory {
                        RankMemoryEvidence::Loading => "Loading allocator samples…".to_string(),
                        RankMemoryEvidence::Unavailable(detail) => format!("Allocator memory unavailable · {detail}"),
                        RankMemoryEvidence::Loaded(_) => "No PyTorch allocator sample reported for this rank.".to_string(),
                    }
                }
            }
        }
    }
}

#[component]
fn DeviceMemoryRow(sample: Option<DeviceMemorySample>, evidence: DeviceMemoryEvidence) -> Element {
    rsx! {
        if let Some(sample) = sample {
            div { class: "mt-2 grid grid-cols-2 gap-x-3 gap-y-2 text-xs sm:grid-cols-4",
                MemoryMetric { label: "Device used", value: format_binary_bytes(sample.current_used_bytes), detail: Some(format!("{} of {}", format_ratio(sample.current_used_bytes, sample.total_bytes), format_binary_bytes(sample.total_bytes))) }
                MemoryMetric { label: "5m sampled peak", value: format_binary_bytes(sample.peak_used_bytes), detail: Some(format!("{} of capacity", format_ratio(sample.peak_used_bytes, sample.total_bytes))) }
                MemoryMetric { label: "Current headroom", value: format_binary_bytes(sample.total_bytes.saturating_sub(sample.current_used_bytes)), detail: Some(format!("GPU {}", sample.device_id)) }
                MemoryMetric { label: "Window samples", value: sample.sample_count.to_string(), detail: Some("latest 5 minutes".to_string()) }
            }
        } else {
            div { class: "mt-2 text-xs text-gray-500",
                match evidence {
                    DeviceMemoryEvidence::Loading => "Loading device-memory samples…".to_string(),
                    DeviceMemoryEvidence::Unavailable(detail) => format!("Device memory unavailable · {detail}"),
                    DeviceMemoryEvidence::Loaded(_) => "No device-memory sample can be attributed to this rank.".to_string(),
                }
            }
        }
    }
}

#[component]
fn MemoryMetric(label: &'static str, value: String, detail: Option<String>) -> Element {
    rsx! {
        div { class: "min-w-0",
            div { class: "text-gray-500", "{label}" }
            div { class: "truncate font-mono text-sm font-semibold tabular-nums text-gray-950", "{value}" }
            if let Some(detail) = detail {
                div { class: "truncate text-gray-500", "{detail}" }
            }
        }
    }
}

#[component]
fn GroupEvidenceRow(
    label: &'static str,
    tone: &'static str,
    members: Vec<i32>,
    communication: GroupCommunicationEvidence,
) -> Element {
    let summary = match &communication {
        GroupCommunicationEvidence::Loaded(samples) => {
            summarize_group_communication(samples, &members)
        }
        GroupCommunicationEvidence::Loading | GroupCommunicationEvidence::Unavailable(_) => None,
    };
    let member_label = format_rank_members(&members);
    let summary_detail = summary.as_ref().map(|summary| {
        format!(
            "{} · {}",
            summary.ops.join(", "),
            format_bytes(summary.total_bytes)
        )
    });
    rsx! {
        div { class: "grid gap-1 py-2 text-xs lg:grid-cols-[minmax(9rem,1fr)_minmax(18rem,2fr)] lg:gap-3",
            div { class: "min-w-0",
                div { class: "font-semibold {tone}", "{label} · {members.len()} ranks" }
                div { class: "mt-0.5 break-words font-mono text-gray-500", "{member_label}" }
            }
            div { class: "min-w-0",
                if let Some(summary) = summary {
                    div { class: "grid grid-cols-4 gap-2 text-gray-700",
                        div { span { class: "block text-gray-500", "Average" } span { class: "font-mono font-semibold tabular-nums text-gray-900", "{summary.avg_ms:.3} ms" } }
                        div { span { class: "block text-gray-500", "Maximum" } span { class: "font-mono font-semibold tabular-nums text-gray-900", "{summary.max_ms:.3} ms" } }
                        div { span { class: "block text-gray-500", "Calls" } span { class: "font-mono font-semibold tabular-nums text-gray-900", "{summary.calls}" } }
                        div { span { class: "block text-gray-500", "Coverage" } span { class: "font-mono font-semibold tabular-nums text-gray-900", "{summary.sampled_ranks}/{members.len()}" } }
                    }
                    div { class: "mt-1 text-gray-500", "{summary_detail.clone().unwrap_or_default()}" }
                } else {
                    match communication {
                        GroupCommunicationEvidence::Loading => rsx! { div { class: "text-gray-500", "Loading exact group samples…" } },
                        GroupCommunicationEvidence::Unavailable(detail) => rsx! { div { class: "text-gray-500", "Group timing unavailable · {detail}" } },
                        GroupCommunicationEvidence::Loaded(_) => rsx! { div { class: "text-gray-500", "No collective sample reported this exact participant set." } },
                    }
                }
            }
        }
    }
}

fn group_communication_evidence(state: Option<&Result<DataFrame>>) -> GroupCommunicationEvidence {
    match state {
        None => GroupCommunicationEvidence::Loading,
        Some(Err(error)) => GroupCommunicationEvidence::Unavailable(error.display_message()),
        Some(Ok(dataframe)) => {
            GroupCommunicationEvidence::Loaded(parse_group_communication(dataframe))
        }
    }
}

fn rank_layout_evidence(state: Option<&Result<DataFrame>>) -> BTreeMap<i32, RankLayoutSummary> {
    let Some(Ok(dataframe)) = state else {
        return BTreeMap::new();
    };
    let index = |name: &str| dataframe.names.iter().position(|column| column == name);
    let (
        Some(recorded_at),
        Some(rank),
        Some(model_chunk),
        Some(layer_start),
        Some(layer_end),
        Some(module_count),
    ) = (
        index("recorded_at_ns"),
        index("rank"),
        index("model_chunk"),
        index("layer_start"),
        index("layer_end"),
        index("module_count"),
    )
    else {
        return BTreeMap::new();
    };
    let mut latest = BTreeMap::<(i32, i32), (i64, i32, i32, usize)>::new();
    for row in dataframe.iter() {
        let Some(rank) = ele_i64(row.get(rank)).and_then(|value| i32::try_from(value).ok()) else {
            continue;
        };
        let Some(chunk) = ele_i64(row.get(model_chunk)).and_then(|value| i32::try_from(value).ok())
        else {
            continue;
        };
        let timestamp = ele_i64(row.get(recorded_at)).unwrap_or_default();
        let start = ele_i64(row.get(layer_start))
            .and_then(|value| i32::try_from(value).ok())
            .unwrap_or(-1);
        let end = ele_i64(row.get(layer_end))
            .and_then(|value| i32::try_from(value).ok())
            .unwrap_or(-1);
        let modules = ele_i64(row.get(module_count))
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or_default();
        let key = (rank, chunk);
        if latest
            .get(&key)
            .is_none_or(|(current, _, _, _)| timestamp > *current)
        {
            latest.insert(key, (timestamp, start, end, modules));
        }
    }
    let ranks_with_chunks = latest
        .keys()
        .filter_map(|(rank, chunk)| (*chunk >= 0).then_some(*rank))
        .collect::<BTreeSet<_>>();
    let mut summaries = BTreeMap::<i32, RankLayoutSummary>::new();
    for ((rank, chunk), (_, start, end, modules)) in latest {
        if chunk < 0 && ranks_with_chunks.contains(&rank) {
            continue;
        }
        let summary = summaries.entry(rank).or_default();
        summary.module_count = summary.module_count.saturating_add(modules);
        if chunk >= 0 {
            summary.model_chunks += 1;
        }
        if start >= 0 {
            summary.layer_start = Some(
                summary
                    .layer_start
                    .map_or(start, |current| current.min(start)),
            );
        }
        if end >= 0 {
            summary.layer_end = Some(summary.layer_end.map_or(end, |current| current.max(end)));
        }
    }
    summaries
}

fn parse_module_catalog(dataframe: &DataFrame) -> Vec<ModuleCatalogEntry> {
    let index = |name: &str| dataframe.names.iter().position(|column| column == name);
    let (
        Some(model_chunk),
        Some(module_path),
        Some(module_type),
        Some(depth),
        Some(local_layer),
        Some(global_layer),
        Some(parameters),
        Some(trainable),
        Some(dtype),
        Some(device),
        Some(is_leaf),
    ) = (
        index("model_chunk"),
        index("module_path"),
        index("module_type"),
        index("depth"),
        index("local_layer_index"),
        index("global_layer_index"),
        index("parameter_count"),
        index("trainable_parameter_count"),
        index("dtype"),
        index("device"),
        index("is_leaf"),
    )
    else {
        return Vec::new();
    };
    dataframe
        .iter()
        .filter_map(|row| {
            let local_layer_index = ele_i64(row.get(local_layer))
                .and_then(|value| i32::try_from(value).ok())
                .filter(|value| *value >= 0);
            let global_layer_index = ele_i64(row.get(global_layer))
                .and_then(|value| i32::try_from(value).ok())
                .filter(|value| *value >= 0);
            Some(ModuleCatalogEntry {
                model_chunk: i32::try_from(ele_i64(row.get(model_chunk))?).ok()?,
                module_path: ele_string(row.get(module_path))?,
                module_type: ele_string(row.get(module_type))?,
                depth: usize::try_from(ele_i64(row.get(depth))?).ok()?,
                local_layer_index,
                global_layer_index,
                parameter_count: u64::try_from(ele_i64(row.get(parameters))?).unwrap_or_default(),
                trainable_parameter_count: u64::try_from(ele_i64(row.get(trainable))?)
                    .unwrap_or_default(),
                dtype: ele_string(row.get(dtype)).unwrap_or_default(),
                device: ele_string(row.get(device)).unwrap_or_default(),
                is_leaf: ele_i64(row.get(is_leaf)).unwrap_or_default() != 0,
            })
        })
        .collect()
}

fn rank_memory_evidence(state: Option<&Result<DataFrame>>) -> RankMemoryEvidence {
    match state {
        None => RankMemoryEvidence::Loading,
        Some(Err(error)) => RankMemoryEvidence::Unavailable(error.display_message()),
        Some(Ok(dataframe)) => RankMemoryEvidence::Loaded(parse_rank_memory(dataframe)),
    }
}

fn parse_rank_memory(dataframe: &DataFrame) -> Vec<RankMemorySample> {
    let index = |name: &str| dataframe.names.iter().position(|column| column == name);
    let (Some(rank), Some(local_step), Some(allocated), Some(max_allocated), Some(cached)) = (
        index("rank"),
        index("local_step"),
        index("allocated"),
        index("max_allocated"),
        index("cached"),
    ) else {
        return Vec::new();
    };
    dataframe
        .iter()
        .filter_map(|row| {
            let allocated_mb = ele_f64(row.get(allocated))?;
            let peak_allocated_mb = ele_f64(row.get(max_allocated))?;
            let reserved_mb = ele_f64(row.get(cached))?;
            if allocated_mb < 0.0 || peak_allocated_mb < 0.0 || reserved_mb < 0.0 {
                return None;
            }
            Some(RankMemorySample {
                rank: i32::try_from(ele_i64(row.get(rank))?).ok()?,
                local_step: ele_i64(row.get(local_step))?,
                allocated_mb,
                peak_allocated_mb,
                reserved_mb,
            })
        })
        .collect()
}

fn device_memory_evidence(state: Option<&Result<DataFrame>>) -> DeviceMemoryEvidence {
    match state {
        None => DeviceMemoryEvidence::Loading,
        Some(Err(error)) => DeviceMemoryEvidence::Unavailable(error.display_message()),
        Some(Ok(dataframe)) => DeviceMemoryEvidence::Loaded(parse_device_memory(dataframe)),
    }
}

fn parse_device_memory(dataframe: &DataFrame) -> Vec<DeviceMemorySample> {
    let index = |name: &str| dataframe.names.iter().position(|column| column == name);
    let rank = index("rank").or_else(|| index("_rank"));
    let (
        Some(device_id),
        Some(current_used_bytes),
        Some(peak_used_bytes),
        Some(total_bytes),
        Some(sample_count),
    ) = (
        index("device_id"),
        index("current_used_bytes"),
        index("peak_used_bytes"),
        index("total_bytes"),
        index("sample_count"),
    )
    else {
        return Vec::new();
    };
    dataframe
        .iter()
        .filter_map(|row| {
            Some(DeviceMemorySample {
                rank: rank
                    .and_then(|rank| ele_i64(row.get(rank)))
                    .and_then(|rank| i32::try_from(rank).ok())
                    .filter(|rank| *rank >= 0),
                device_id: i32::try_from(ele_i64(row.get(device_id))?).ok()?,
                current_used_bytes: u64::try_from(ele_i64(row.get(current_used_bytes))?).ok()?,
                peak_used_bytes: u64::try_from(ele_i64(row.get(peak_used_bytes))?).ok()?,
                total_bytes: u64::try_from(ele_i64(row.get(total_bytes))?).ok()?,
                sample_count: usize::try_from(ele_i64(row.get(sample_count))?).ok()?,
            })
        })
        .collect()
}

fn select_device_memory_sample(
    evidence: &DeviceMemoryEvidence,
    rank: Option<i32>,
    local_rank: Option<i32>,
    local_rank_confirmed: bool,
) -> Option<DeviceMemorySample> {
    let DeviceMemoryEvidence::Loaded(samples) = evidence else {
        return None;
    };
    let device_id = local_rank?;
    if let Some(rank) = rank {
        if let Some(sample) = samples
            .iter()
            .find(|sample| sample.rank == Some(rank) && sample.device_id == device_id)
        {
            return Some(sample.clone());
        }
    }
    (!samples.iter().any(|sample| sample.rank.is_some()) && local_rank_confirmed)
        .then(|| {
            samples
                .iter()
                .find(|sample| sample.device_id == device_id)
                .cloned()
        })
        .flatten()
}

fn parse_group_communication(dataframe: &DataFrame) -> Vec<GroupCommunicationSample> {
    let index = |name: &str| dataframe.names.iter().position(|column| column == name);
    let (
        Some(rank),
        Some(op),
        Some(group_size),
        Some(participants),
        Some(calls),
        Some(avg_ms),
        Some(max_ms),
        Some(total_bytes),
    ) = (
        index("rank"),
        index("op"),
        index("group_size"),
        index("participate_ranks"),
        index("calls"),
        index("avg_ms"),
        index("max_ms"),
        index("total_bytes"),
    )
    else {
        return Vec::new();
    };
    dataframe
        .iter()
        .filter_map(|row| {
            let mut participants =
                serde_json::from_str::<Vec<i32>>(&ele_string(row.get(participants))?).ok()?;
            participants.sort_unstable();
            participants.dedup();
            Some(GroupCommunicationSample {
                rank: i32::try_from(ele_i64(row.get(rank))?).ok()?,
                op: ele_string(row.get(op))?,
                group_size: usize::try_from(ele_i64(row.get(group_size))?).ok()?,
                participants,
                calls: usize::try_from(ele_i64(row.get(calls))?).ok()?,
                avg_ms: ele_f64(row.get(avg_ms))?,
                max_ms: ele_f64(row.get(max_ms))?,
                total_bytes: u64::try_from(ele_i64(row.get(total_bytes))?).unwrap_or(0),
            })
        })
        .collect()
}

fn placement_group_members(
    placement: &PlacementModel,
    active: &PlacementProcess,
    group: PlacementGroup,
) -> Vec<i32> {
    let mut members = placement
        .hosts
        .iter()
        .flat_map(|host| host.processes.iter())
        .filter(|candidate| {
            matches!(
                placement_group_membership(active, candidate),
                Some(PlacementGroup::Focus)
            ) || placement_group_membership(active, candidate) == Some(group)
        })
        .filter_map(|candidate| candidate.rank)
        .collect::<Vec<_>>();
    members.sort_unstable();
    members.dedup();
    members
}

fn summarize_group_communication(
    samples: &[GroupCommunicationSample],
    members: &[i32],
) -> Option<GroupCommunicationSummary> {
    let mut expected = members.to_vec();
    expected.sort_unstable();
    expected.dedup();
    let matching = samples
        .iter()
        .filter(|sample| sample.group_size == expected.len() && sample.participants == expected)
        .collect::<Vec<_>>();
    if matching.is_empty() {
        return None;
    }
    let calls = matching.iter().map(|sample| sample.calls).sum::<usize>();
    if calls == 0 {
        return None;
    }
    let avg_ms = matching
        .iter()
        .map(|sample| sample.avg_ms * sample.calls as f64)
        .sum::<f64>()
        / calls as f64;
    let max_ms = matching
        .iter()
        .map(|sample| sample.max_ms)
        .max_by(f64::total_cmp)
        .unwrap_or(0.0);
    let total_bytes = matching.iter().fold(0u64, |total, sample| {
        total.saturating_add(sample.total_bytes)
    });
    let sampled_ranks = matching
        .iter()
        .map(|sample| sample.rank)
        .collect::<BTreeSet<_>>()
        .len();
    let ops = matching
        .iter()
        .map(|sample| sample.op.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    Some(GroupCommunicationSummary {
        calls,
        avg_ms,
        max_ms,
        total_bytes,
        sampled_ranks,
        ops,
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

fn ele_f64(value: Option<&Ele>) -> Option<f64> {
    match value? {
        Ele::F64(value) => Some(*value),
        Ele::F32(value) => Some(f64::from(*value)),
        Ele::I64(value) => Some(*value as f64),
        Ele::I32(value) => Some(f64::from(*value)),
        Ele::Text(value) => value.parse().ok(),
        _ => None,
    }
}

fn ele_string(value: Option<&Ele>) -> Option<String> {
    match value? {
        Ele::Text(value) | Ele::Url(value) => Some(value.clone()),
        _ => None,
    }
}

fn format_delta_percent(value: f64, reference: f64) -> String {
    format!("{:+.1}%", (value / reference - 1.0) * 100.0)
}

fn format_memory_mb(value: f64) -> String {
    if value >= 1024.0 {
        format!("{:.2} GiB", value / 1024.0)
    } else {
        format!("{value:.0} MiB")
    }
}

fn format_memory_ratio(allocated_mb: f64, reserved_mb: f64) -> String {
    if reserved_mb > 0.0 {
        format!("{:.1}%", allocated_mb / reserved_mb * 100.0)
    } else {
        "—".to_string()
    }
}

fn format_parameter_count(count: u64) -> String {
    if count >= 1_000_000_000 {
        format!("{:.2}B", count as f64 / 1_000_000_000.0)
    } else if count >= 1_000_000 {
        format!("{:.2}M", count as f64 / 1_000_000.0)
    } else if count >= 1_000 {
        format!("{:.1}K", count as f64 / 1_000.0)
    } else {
        count.to_string()
    }
}

fn format_binary_bytes(bytes: u64) -> String {
    format_bytes(bytes)
}

fn format_ratio(value: u64, total: u64) -> String {
    if total > 0 {
        format!("{:.1}%", value as f64 / total as f64 * 100.0)
    } else {
        "—".to_string()
    }
}

fn format_rank_members(members: &[i32]) -> String {
    const LIMIT: usize = 12;
    let mut label = members
        .iter()
        .take(LIMIT)
        .map(|rank| format!("R{rank}"))
        .collect::<Vec<_>>()
        .join(" ");
    if members.len() > LIMIT {
        label.push_str(&format!(" +{}", members.len() - LIMIT));
    }
    label
}

fn format_bytes(bytes: u64) -> String {
    match bytes {
        value if value >= 1 << 30 => format!("{:.1} GiB", value as f64 / (1u64 << 30) as f64),
        value if value >= 1 << 20 => format!("{:.1} MiB", value as f64 / (1u64 << 20) as f64),
        value if value >= 1 << 10 => format!("{:.1} KiB", value as f64 / (1u64 << 10) as f64),
        value => format!("{value} B"),
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
        return "not reported".to_string();
    }
    let seconds = now_micros.saturating_sub(timestamp) / 1_000_000;
    match seconds {
        0 => "<1s ago".to_string(),
        1..=59 => format!("{seconds}s ago"),
        60..=3_599 => format!("{}m ago", seconds / 60),
        _ => format!("{}h ago", seconds / 3_600),
    }
}

#[component]
fn PlacementCell(
    host: String,
    process: PlacementProcess,
    active: Option<PlacementProcess>,
    memory: Option<DeviceMemorySample>,
    mut hovered_rank: Signal<Option<i32>>,
    mut tooltip_rank: Signal<Option<i32>>,
    mut tooltip_position: Signal<PlacementTooltipPosition>,
    local_step: Option<i64>,
) -> Element {
    let rank = process.rank;
    let local_device_id = process.local_rank;
    let rank_label = rank
        .map(|value| format!("R{value}"))
        .unwrap_or_else(|| "R?".to_string());
    let local_rank = process
        .local_rank
        .map(|value| value.to_string())
        .unwrap_or_else(|| "?".to_string());
    let status = process.status.as_deref().unwrap_or("unknown");
    let role = process.role_label.as_deref().unwrap_or("rank");
    let coordinates = process
        .coordinates
        .iter()
        .map(|(dimension, value)| {
            format!(
                "{}{}",
                dimension.chars().next().unwrap_or('?').to_ascii_uppercase(),
                value
            )
        })
        .collect::<Vec<_>>()
        .join(" ");
    let group = active
        .as_ref()
        .and_then(|active| placement_group_membership(active, &process));
    let cell_class = cell_classes(group, process.status.as_deref());
    let group_name = placement_group_name(group);
    let memory_style = memory_heat_style(memory.as_ref(), group);
    let memory_detail = memory
        .as_ref()
        .map(|sample| {
            format!(
                " · memory {} current, {} window peak, {} capacity",
                format_binary_bytes(sample.current_used_bytes),
                format_binary_bytes(sample.peak_used_bytes),
                format_binary_bytes(sample.total_bytes),
            )
        })
        .unwrap_or_else(|| " · memory not reported".to_string());
    let coordinate_detail = if coordinates.is_empty() {
        String::new()
    } else {
        format!(" · {coordinates}")
    };
    let layout_detail = process
        .layout
        .as_ref()
        .map(|layout| format!(" · {}", format_rank_layout(layout)))
        .unwrap_or_default();
    let title = format!(
        "{rank_label} · {host} · GPU{local_rank} · {status} · {role}{}{}{}{}",
        coordinate_detail,
        group_name
            .map(|name| format!(" · {name}"))
            .unwrap_or_default(),
        memory_detail,
        layout_detail,
    );
    let pinned_host = host.clone();
    let pinned = INVESTIGATION_CONTEXT.read().rank == rank;
    let cell_text = placement_group_code(group).unwrap_or(local_rank.as_str());

    rsx! {
        button {
            r#type: "button",
            class: "flex h-6 w-6 items-center justify-center rounded-[3px] border font-mono text-xs font-semibold transition-colors focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-700 focus-visible:ring-offset-1 {cell_class}",
            style: "{memory_style}",
            aria_label: "{title}",
            aria_pressed: pinned.to_string(),
            title: "{title}",
            onmouseover: move |_| hovered_rank.set(rank),
            onfocus: move |_| hovered_rank.set(rank),
            onclick: move |event: MouseEvent| {
                event.stop_propagation();
                hovered_rank.set(rank);
                if let Some(rank) = rank {
                    let coordinates = event.client_coordinates();
                    set_training_rank_context(rank, local_step, Some(&pinned_host));
                    if let Some(device_id) = local_device_id {
                        set_memory_device_context(Some(rank), Some(&pinned_host), device_id);
                    }
                    tooltip_position.set(PlacementTooltipPosition {
                        x: coordinates.x,
                        y: coordinates.y,
                    });
                    tooltip_rank.set(Some(rank));
                }
            },
            onblur: move |_| hovered_rank.set(None),
            "{cell_text}"
        }
    }
}

fn placement_group_code(group: Option<PlacementGroup>) -> Option<&'static str> {
    match group {
        Some(PlacementGroup::Focus) => Some("●"),
        Some(PlacementGroup::Tensor) => Some("T"),
        Some(PlacementGroup::Data) => Some("D"),
        Some(PlacementGroup::Pipeline) => Some("P"),
        None => None,
    }
}

fn placement_group_name(group: Option<PlacementGroup>) -> Option<&'static str> {
    match group {
        Some(PlacementGroup::Focus) => Some("selected GPU"),
        Some(PlacementGroup::Tensor) => Some("tensor-parallel peer"),
        Some(PlacementGroup::Data) => Some("data-parallel peer"),
        Some(PlacementGroup::Pipeline) => Some("pipeline-parallel peer"),
        None => None,
    }
}

#[component]
fn GroupLegend(label: &'static str, count: Option<usize>, class: &'static str) -> Element {
    rsx! {
        span { class: "flex items-center gap-1",
            span { class: "h-3 w-3 rounded-[2px] border border-dashed {class}" }
            "{label}"
            if let Some(count) = count {
                span { class: "font-mono font-semibold text-gray-700", "{count}" }
            }
        }
    }
}

#[component]
fn MemoryLegend(available: bool) -> Element {
    let style = if available {
        "background: linear-gradient(90deg, hsl(261 84% 97%), hsl(261 70% 52%));"
    } else {
        "background: #f3f4f6;"
    };
    rsx! {
        span { class: "flex items-center gap-1.5",
            span {
                class: "h-2.5 w-12 rounded-sm border border-violet-200",
                style: "{style}",
            }
            if available { "Current / capacity 0–100%" } else { "Memory unavailable" }
        }
    }
}

fn coordinate<'a>(process: &'a PlacementProcess, dimension: &str) -> Option<&'a str> {
    process
        .coordinates
        .iter()
        .find_map(|(key, value)| (key == dimension).then_some(value.as_str()))
}

fn coordinate_i32(process: &PlacementProcess, dimension: &str) -> Option<i32> {
    coordinate(process, dimension)?.parse().ok()
}

fn format_rank_layout(layout: &RankLayoutSummary) -> String {
    let layers = match (layout.layer_start, layout.layer_end) {
        (Some(start), Some(end)) if start == end => format!("L{start}"),
        (Some(start), Some(end)) => format!("L{start}–{end}"),
        _ => "layers unknown".to_string(),
    };
    let chunks = (layout.model_chunks > 1).then(|| format!(" · {} chunks", layout.model_chunks));
    format!(
        "{layers} · {} modules{}",
        layout.module_count,
        chunks.unwrap_or_default()
    )
}

fn placement_group_membership(
    active: &PlacementProcess,
    candidate: &PlacementProcess,
) -> Option<PlacementGroup> {
    if active.rank == candidate.rank {
        return Some(PlacementGroup::Focus);
    }

    let active_dp = coordinate(active, "dp")?;
    let active_pp = coordinate(active, "pp")?;
    let active_tp = coordinate(active, "tp")?;
    let candidate_dp = coordinate(candidate, "dp")?;
    let candidate_pp = coordinate(candidate, "pp")?;
    let candidate_tp = coordinate(candidate, "tp")?;

    if active_dp == candidate_dp && active_pp == candidate_pp {
        Some(PlacementGroup::Tensor)
    } else if active_pp == candidate_pp && active_tp == candidate_tp {
        Some(PlacementGroup::Data)
    } else if active_dp == candidate_dp && active_tp == candidate_tp {
        Some(PlacementGroup::Pipeline)
    } else {
        None
    }
}

fn placement_group_sizes(
    placement: &PlacementModel,
    active: &PlacementProcess,
) -> Option<PlacementGroupSizes> {
    coordinate(active, "dp")?;
    coordinate(active, "pp")?;
    coordinate(active, "tp")?;

    let mut sizes = PlacementGroupSizes {
        tensor: 1,
        data: 1,
        pipeline: 1,
    };
    for candidate in placement
        .hosts
        .iter()
        .flat_map(|host| host.processes.iter())
    {
        match placement_group_membership(active, candidate) {
            Some(PlacementGroup::Tensor) => sizes.tensor += 1,
            Some(PlacementGroup::Data) => sizes.data += 1,
            Some(PlacementGroup::Pipeline) => sizes.pipeline += 1,
            Some(PlacementGroup::Focus) | None => {}
        }
    }
    Some(sizes)
}

fn cell_classes(group: Option<PlacementGroup>, status: Option<&str>) -> &'static str {
    match group {
        Some(PlacementGroup::Focus) => {
            "border-blue-700 bg-blue-600 text-white ring-2 ring-blue-200"
        }
        Some(PlacementGroup::Tensor) => "border-dashed border-violet-500 bg-white text-violet-900",
        Some(PlacementGroup::Data) => "border-dashed border-emerald-500 bg-white text-emerald-900",
        Some(PlacementGroup::Pipeline) => "border-dashed border-amber-500 bg-white text-amber-900",
        None if matches!(
            status.unwrap_or_default().to_ascii_lowercase().as_str(),
            "failed" | "error" | "offline" | "unhealthy"
        ) =>
        {
            "border-red-400 bg-red-100 text-red-800"
        }
        None => "border-gray-300 bg-gray-100 text-gray-500 hover:border-blue-400 hover:bg-blue-50",
    }
}

fn placement_device_memory(
    evidence: &DeviceMemoryEvidence,
    process: &PlacementProcess,
) -> Option<DeviceMemorySample> {
    let DeviceMemoryEvidence::Loaded(samples) = evidence else {
        return None;
    };
    let device_id = process.local_rank?;
    if let Some(rank) = process.rank {
        return samples
            .iter()
            .find(|sample| sample.rank == Some(rank) && sample.device_id == device_id)
            .cloned();
    }
    (!samples.iter().any(|sample| sample.rank.is_some()))
        .then(|| {
            samples
                .iter()
                .find(|sample| sample.device_id == device_id)
                .cloned()
        })
        .flatten()
}

fn memory_heat_style(sample: Option<&DeviceMemorySample>, group: Option<PlacementGroup>) -> String {
    if matches!(group, Some(PlacementGroup::Focus)) {
        return String::new();
    }
    let Some(sample) = sample.filter(|sample| sample.total_bytes > 0) else {
        return String::new();
    };
    let percent =
        (sample.current_used_bytes as f64 / sample.total_bytes as f64 * 100.0).clamp(0.0, 100.0);
    let lightness = 97.0 - percent * 0.45;
    let text = if percent >= 66.0 {
        "#ffffff"
    } else {
        "#4c1d95"
    };
    format!("background-color: hsl(261 70% {lightness:.1}%); color: {text};")
}

fn parse_coordinates(role: Option<&str>) -> Vec<(String, String)> {
    let mut parsed = BTreeMap::new();
    for part in role.unwrap_or_default().split(',') {
        let Some((key, value)) = part.split_once('=') else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim();
        if matches!(key.as_str(), "dp" | "pp" | "tp" | "sp" | "cp" | "ep") && !value.is_empty() {
            parsed.insert(key, value.to_string());
        }
    }
    ["dp", "pp", "tp", "sp", "cp", "ep"]
        .into_iter()
        .filter_map(|key| parsed.remove(key).map(|value| (key.to_string(), value)))
        .collect()
}

fn build_placement(nodes: &[Node], layouts: &BTreeMap<i32, RankLayoutSummary>) -> PlacementModel {
    let mut hosts: BTreeMap<String, Vec<PlacementProcess>> = BTreeMap::new();
    let mut ranks = BTreeSet::new();
    let mut expected_ranks = 0usize;

    for node in nodes {
        if let Some(rank) = node.rank {
            ranks.insert(rank);
        }
        if let Some(world_size) = node.world_size.filter(|size| *size > 0) {
            expected_ranks = expected_ranks.max(world_size as usize);
        }
        let coordinates = parse_coordinates(node.role.as_deref());
        let role_label = node
            .role_name
            .clone()
            .filter(|role| !role.trim().is_empty());
        let host = if node.host.trim().is_empty() {
            "Unknown host".to_string()
        } else {
            node.host.clone()
        };
        hosts.entry(host).or_default().push(PlacementProcess {
            rank: node.rank,
            local_rank: node.local_rank,
            addr: node.addr.clone(),
            role_label,
            coordinates,
            status: node.status.clone(),
            timestamp: node.timestamp,
            layout: node.rank.and_then(|rank| layouts.get(&rank).cloned()),
        });
    }

    let mut hosts = hosts
        .into_iter()
        .map(|(name, mut processes)| {
            processes.sort_by_key(|process| {
                (
                    process.rank.unwrap_or(i32::MAX),
                    process.local_rank.unwrap_or(i32::MAX),
                )
            });
            PlacementHost { name, processes }
        })
        .collect::<Vec<_>>();
    hosts.sort_by(|left, right| left.name.cmp(&right.name));

    let observed_ranks = ranks.len();
    if expected_ranks == 0 {
        expected_ranks = observed_ranks;
    }
    let mut values: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for process in hosts.iter().flat_map(|host| host.processes.iter()) {
        for (dimension, value) in &process.coordinates {
            values
                .entry(dimension.clone())
                .or_default()
                .insert(value.clone());
        }
    }
    let parallel_sizes = ["dp", "pp", "tp", "sp", "cp", "ep"]
        .into_iter()
        .filter_map(|dimension| {
            values
                .get(dimension)
                .map(|values| (dimension.to_string(), values.len()))
        })
        .collect::<Vec<_>>();
    let has_parallel_coordinates = hosts
        .iter()
        .flat_map(|host| host.processes.iter())
        .any(|process| !process.coordinates.is_empty());

    PlacementModel {
        hosts,
        observed_ranks,
        expected_ranks,
        has_parallel_coordinates,
        parallel_sizes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use probing_proto::prelude::Seq;

    #[test]
    fn placement_uses_reported_megatron_coordinates() {
        let nodes = (0..64)
            .map(|rank| Node {
                host: format!("host-{}", rank / 8),
                rank: Some(rank),
                local_rank: Some(rank % 8),
                world_size: Some(64),
                role: Some(format!(
                    "dp={},pp={},tp={}",
                    rank / 8,
                    (rank / 2) % 4,
                    rank % 2
                )),
                ..Default::default()
            })
            .collect::<Vec<_>>();
        let placement = build_placement(&nodes, &BTreeMap::new());

        assert_eq!(placement.hosts.len(), 8);
        assert_eq!(placement.observed_ranks, 64);
        assert_eq!(placement.expected_ranks, 64);
        assert_eq!(
            placement.parallel_sizes,
            vec![("dp".into(), 8), ("pp".into(), 4), ("tp".into(), 2)]
        );

        let active = placement
            .hosts
            .iter()
            .flat_map(|host| host.processes.iter())
            .find(|process| process.rank == Some(0))
            .unwrap();
        assert_eq!(
            placement_group_members(&placement, active, PlacementGroup::Tensor),
            vec![0, 1]
        );
        assert_eq!(
            placement_group_members(&placement, active, PlacementGroup::Pipeline),
            vec![0, 2, 4, 6]
        );
        assert_eq!(
            placement_group_members(&placement, active, PlacementGroup::Data),
            vec![0, 8, 16, 24, 32, 40, 48, 56]
        );
    }

    #[test]
    fn communication_groups_have_non_color_labels() {
        assert_eq!(placement_group_code(Some(PlacementGroup::Focus)), Some("●"));
        assert_eq!(
            placement_group_code(Some(PlacementGroup::Tensor)),
            Some("T")
        );
        assert_eq!(placement_group_code(Some(PlacementGroup::Data)), Some("D"));
        assert_eq!(
            placement_group_code(Some(PlacementGroup::Pipeline)),
            Some("P")
        );
        assert_eq!(placement_group_code(None), None);
    }

    #[test]
    fn pinned_state_follows_selected_rank_not_pointer_presence() {
        assert!(placement_selection_is_pinned(Some(7), Some(7)));
        assert!(!placement_selection_is_pinned(Some(8), Some(7)));
        assert!(!placement_selection_is_pinned(None, None));
    }

    #[test]
    fn communication_summary_requires_exact_participant_membership() {
        let samples = vec![
            GroupCommunicationSample {
                rank: 0,
                op: "all_gather".into(),
                group_size: 2,
                participants: vec![0, 1],
                calls: 3,
                avg_ms: 1.0,
                max_ms: 1.5,
                total_bytes: 300,
            },
            GroupCommunicationSample {
                rank: 1,
                op: "all_gather".into(),
                group_size: 2,
                participants: vec![0, 1],
                calls: 1,
                avg_ms: 2.0,
                max_ms: 2.5,
                total_bytes: 100,
            },
        ];
        let summary = summarize_group_communication(&samples, &[0, 1]).unwrap();
        assert_eq!(summary.calls, 4);
        assert_eq!(summary.sampled_ranks, 2);
        assert_eq!(summary.avg_ms, 1.25);
        assert_eq!(summary.max_ms, 2.5);
        assert_eq!(summary.total_bytes, 400);
        assert!(summarize_group_communication(&samples, &[0, 2]).is_none());
    }

    #[test]
    fn rank_memory_keeps_current_peak_and_reserved_semantics_separate() {
        let dataframe = DataFrame::new(
            vec![
                "rank".into(),
                "local_step".into(),
                "allocated".into(),
                "max_allocated".into(),
                "cached".into(),
            ],
            vec![
                Seq::SeqI32(vec![7]),
                Seq::SeqI64(vec![119]),
                Seq::SeqF64(vec![12_288.0]),
                Seq::SeqF64(vec![16_384.0]),
                Seq::SeqF64(vec![20_480.0]),
            ],
        );
        let samples = parse_rank_memory(&dataframe);

        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].rank, 7);
        assert_eq!(samples[0].local_step, 119);
        assert_eq!(samples[0].allocated_mb, 12_288.0);
        assert_eq!(samples[0].peak_allocated_mb, 16_384.0);
        assert_eq!(samples[0].reserved_mb, 20_480.0);
        assert_eq!(format_memory_mb(samples[0].allocated_mb), "12.00 GiB");
        assert_eq!(format_memory_ratio(12_288.0, 20_480.0), "60.0%");
    }

    #[test]
    fn device_memory_is_attributed_by_rank_and_local_device() {
        let gib = 1_i64 << 30;
        let dataframe = DataFrame::new(
            vec![
                "device_id".into(),
                "current_used_bytes".into(),
                "peak_used_bytes".into(),
                "total_bytes".into(),
                "sample_count".into(),
                "_rank".into(),
            ],
            vec![
                Seq::SeqI32(vec![1, 1]),
                Seq::SeqI64(vec![40 * gib, 48 * gib]),
                Seq::SeqI64(vec![60 * gib, 64 * gib]),
                Seq::SeqI64(vec![80 * gib, 80 * gib]),
                Seq::SeqI64(vec![300, 300]),
                Seq::SeqI32(vec![1, 57]),
            ],
        );
        let evidence = DeviceMemoryEvidence::Loaded(parse_device_memory(&dataframe));
        let selected = select_device_memory_sample(&evidence, Some(57), Some(1), false).unwrap();

        assert_eq!(selected.rank, Some(57));
        assert_eq!(selected.current_used_bytes, 48 * gib as u64);
        assert_eq!(selected.peak_used_bytes, 64 * gib as u64);
        assert_eq!(
            format_ratio(selected.current_used_bytes, selected.total_bytes),
            "60.0%"
        );
        assert_eq!(format_binary_bytes(selected.total_bytes), "80.0 GiB");
    }

    #[test]
    fn memory_heat_uses_current_device_pressure_without_hiding_focus() {
        let sample = DeviceMemorySample {
            rank: Some(57),
            device_id: 1,
            current_used_bytes: 80,
            peak_used_bytes: 90,
            total_bytes: 100,
            sample_count: 300,
        };

        let heat = memory_heat_style(Some(&sample), None);
        assert!(heat.contains("61.0%"));
        assert!(heat.contains("#ffffff"));
        assert!(memory_heat_style(Some(&sample), Some(PlacementGroup::Focus)).is_empty());
        assert!(memory_heat_style(None, None).is_empty());
    }

    #[test]
    fn rank_layout_aggregates_model_chunks_and_ignores_initial_snapshot() {
        let dataframe = DataFrame::new(
            vec![
                "recorded_at_ns".into(),
                "rank".into(),
                "model_chunk".into(),
                "layer_start".into(),
                "layer_end".into(),
                "module_count".into(),
            ],
            vec![
                Seq::SeqI64(vec![1, 2, 3]),
                Seq::SeqI32(vec![7, 7, 7]),
                Seq::SeqI32(vec![-1, 0, 1]),
                Seq::SeqI32(vec![-1, 0, 8]),
                Seq::SeqI32(vec![-1, 7, 15]),
                Seq::SeqI64(vec![0, 120, 130]),
            ],
        );
        let state = Ok(dataframe);
        let layouts = rank_layout_evidence(Some(&state));
        let layout = layouts.get(&7).unwrap();
        assert_eq!(layout.layer_start, Some(0));
        assert_eq!(layout.layer_end, Some(15));
        assert_eq!(layout.module_count, 250);
        assert_eq!(layout.model_chunks, 2);
        assert_eq!(format_rank_layout(layout), "L0–15 · 250 modules · 2 chunks");
    }

    #[test]
    fn module_catalog_preserves_layer_and_parameter_metadata() {
        let dataframe = DataFrame::new(
            vec![
                "model_chunk".into(),
                "module_path".into(),
                "module_type".into(),
                "depth".into(),
                "local_layer_index".into(),
                "global_layer_index".into(),
                "parameter_count".into(),
                "trainable_parameter_count".into(),
                "dtype".into(),
                "device".into(),
                "is_leaf".into(),
            ],
            vec![
                Seq::SeqI32(vec![0]),
                Seq::SeqText(vec!["decoder.layers.0.mlp".into()]),
                Seq::SeqText(vec!["megatron.MLP".into()]),
                Seq::SeqI64(vec![4]),
                Seq::SeqI32(vec![0]),
                Seq::SeqI32(vec![8]),
                Seq::SeqI64(vec![1_048_576]),
                Seq::SeqI64(vec![1_048_576]),
                Seq::SeqText(vec!["torch.float16".into()]),
                Seq::SeqText(vec!["cuda:0".into()]),
                Seq::SeqI32(vec![1]),
            ],
        );
        let entries = parse_module_catalog(&dataframe);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].global_layer_index, Some(8));
        assert_eq!(entries[0].parameter_count, 1_048_576);
        assert!(entries[0].is_leaf);
        assert_eq!(format_parameter_count(entries[0].parameter_count), "1.05M");
        assert!(module_matches_filter(&entries[0], "mlp"));
        assert!(module_matches_filter(&entries[0], "float16"));
        assert!(module_matches_filter(&entries[0], "l8"));
        assert!(!module_matches_filter(&entries[0], "attention"));
    }
}
