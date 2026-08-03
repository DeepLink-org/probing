//! Training observability: local step/collective views + on-demand cluster scan.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use dioxus::prelude::*;
use probing_proto::prelude::Node;

use crate::agent::load_skill;
use crate::api::{ApiClient, ClusterQueryResponse, StepDurationSample, StepMatrixResponse};
use crate::components::card::Card;
use crate::components::collapsible_card::CollapsibleCardWithIcon;
use crate::components::common::{AppErrorDisplay, AsyncBoundary, EmptyState, LoadingState};
use crate::components::dataframe_view::DataFrameView;
use crate::components::icon::Icon;
use crate::components::page::{PageContainer, PageTitle};
use crate::components::poll_status::{PollStatusBar, RefreshButton};
use crate::components::workspace::{ChipButton, WidthSegment};
use crate::hooks::{use_app_resource, use_page_visible, use_poll_tick_gated};
use crate::state::agent::{AGENT_INPUT, AGENT_PANEL_OPEN};
use crate::state::investigation::{apply_context_from_dataframe_row, set_training_step_context};
use crate::state::training::{
    placement_availability, TRAINING_CLUSTER_SCOPE, TRAINING_PLACEMENT_AVAILABILITY,
    TRAINING_REFRESH,
};
use crate::state::ui_tasks::ui_agent_busy;
use crate::utils::error::AppError;

const POLL_MS: u32 = 5000;
const STEP_LIMIT: usize = 120;
const COMM_LIMIT: usize = 30;
const STEP_CARD_TITLE: &str = "Step time";

const COMM_SQL: &str = "SELECT local_step, rank, op, group_size, duration_ms, bytes, role \
     FROM python.comm_collective ORDER BY timestamp DESC LIMIT ";

const COMM_SUMMARY_SQL: &str = "SELECT op, count(*) AS n, \
     round(avg(duration_ms), 2) AS avg_ms, round(max(duration_ms), 2) AS max_ms, \
     sum(bytes) AS total_bytes \
     FROM python.comm_collective GROUP BY op ORDER BY avg_ms DESC LIMIT 10";

const MODULE_HOTSPOTS_SQL: &str = "SELECT module, stage, count(DISTINCT local_step) AS steps, \
     count(*) AS hooks, round(avg(duration), 4) AS avg_sec, round(sum(duration), 4) AS total_sec \
     FROM python.torch_trace \
     WHERE local_step >= GREATEST(COALESCE((SELECT max(local_step) FROM python.torch_trace), 0) - 9, 1) \
       AND stage LIKE 'post %' AND duration > 0 \
       AND module IS NOT NULL AND module != '' AND module != 'None' \
     GROUP BY module, stage ORDER BY total_sec DESC LIMIT 12";

const STEP_PHASE_SQL: &str = "SELECT local_step, \
     round(sum(CASE WHEN stage = 'post forward' THEN duration ELSE 0 END), 4) AS forward_sec, \
     round(sum(CASE WHEN stage = 'post step' THEN duration ELSE 0 END), 4) AS optim_sec \
     FROM python.torch_trace \
     WHERE local_step >= GREATEST(COALESCE((SELECT max(local_step) FROM python.torch_trace), 0) - 15, 1) \
       AND stage LIKE 'post %' AND duration > 0 \
       AND module IS NOT NULL AND module != '' AND module != 'None' \
     GROUP BY local_step ORDER BY local_step";

const QUICK_SKILLS: &[(&str, &str)] = &[
    ("slow_rank", "Slow rank"),
    ("nccl_culprit_victim", "NCCL"),
    ("comm_bottleneck", "Comm"),
    ("module_bottleneck", "Bottleneck"),
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum DataScope {
    Local,
    Cluster,
}

#[derive(Clone, Debug)]
struct ClusterScanOutput {
    matrix: Result<StepMatrixResponse, AppError>,
    comm: Result<ClusterQueryResponse, AppError>,
    nodes_failed: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
struct HeatCell {
    duration_ms: f64,
    outlier: bool,
}

/// A training step selected in the step chart (display index + trace coordinate).
#[derive(Clone, Debug, PartialEq)]
struct SelectedStep {
    rank: i32,
    display_step: i64,
    coord_step: i64,
    duration_ms: f64,
}

#[derive(Clone, Debug, PartialEq)]
struct StepPoint {
    display_step: i64,
    coord_step: i64,
    duration_ms: f64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PlacementProcess {
    rank: Option<i32>,
    local_rank: Option<i32>,
    role_label: Option<String>,
    coordinates: Vec<(String, String)>,
    status: Option<String>,
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

fn trace_step(coord: i64, display: i64) -> i64 {
    if coord >= 0 {
        coord
    } else {
        display
    }
}

fn step_module_sql(coord_step: i64) -> String {
    format!(
        "SELECT module, stage, round(duration, 4) AS sec \
         FROM python.torch_trace \
         WHERE local_step = {coord_step} AND stage LIKE 'post %' AND duration > 0 \
           AND module IS NOT NULL AND module != '' AND module != 'None' \
         ORDER BY duration DESC LIMIT 12"
    )
}

fn step_span_sql(display_step: i64) -> String {
    format!(
        "SELECT s.name, s.phase, round((CAST(e.time AS BIGINT) - CAST(s.time AS BIGINT)) / 1000000.0, 2) AS duration_ms \
         FROM python.trace_event s \
         JOIN python.trace_event e ON s.span_id = e.span_id AND e.record_type = 'span_end' \
         WHERE s.record_type = 'span_start' AND s.name != 'train.step' \
           AND s.attributes LIKE '%\"local_step\":{display_step}%' \
         ORDER BY duration_ms DESC LIMIT 12"
    )
}

fn select_training_step(
    rank: i32,
    display_step: i64,
    coord_step: i64,
    duration_ms: f64,
    mut selected: Signal<Option<SelectedStep>>,
) {
    selected.set(Some(SelectedStep {
        rank,
        display_step,
        coord_step,
        duration_ms,
    }));
    set_training_step_context(rank, Some(display_step), None);
}

#[component]
pub fn Training(#[props(default = true)] show_controls: bool) -> Element {
    let visible = use_page_visible();
    let poll = use_poll_tick_gated(POLL_MS, Some(visible));
    let local_tick = poll().wrapping_add(*TRAINING_REFRESH.read());

    let nodes = use_app_resource(|| {
        let _ = *TRAINING_REFRESH.read();
        async move { ApiClient::new().get_nodes().await }
    });
    use_effect(move || {
        *TRAINING_PLACEMENT_AVAILABILITY.write() = placement_availability(nodes().as_ref());
    });
    let mut cluster_scan = use_action(|| async move {
        let client = ApiClient::new();
        let matrix_res = client.fetch_step_matrix(STEP_LIMIT, true).await;
        let comm_res = client
            .cluster_query(&format!("{COMM_SQL}{COMM_LIMIT}"), true)
            .await;

        let mut failed: HashSet<String> = HashSet::new();
        if let Ok(ref m) = matrix_res {
            failed.extend(m.nodes_failed.iter().cloned());
        }
        if let Ok(ref c) = comm_res {
            failed.extend(c.meta.nodes_failed.iter().cloned());
        }
        let mut merged: Vec<String> = failed.into_iter().collect();
        merged.sort();

        Ok::<ClusterScanOutput, AppError>(ClusterScanOutput {
            matrix: matrix_res,
            comm: comm_res,
            nodes_failed: merged,
        })
    });

    use_effect(move || {
        let refresh = *TRAINING_REFRESH.read();
        if *TRAINING_CLUSTER_SCOPE.read() {
            let _ = refresh;
            cluster_scan.call();
        }
    });

    let node_state = nodes();
    let peer_count = node_state
        .as_ref()
        .and_then(|r| r.as_ref().ok())
        .map(|nodes| nodes.len().saturating_sub(1))
        .unwrap_or(0);

    let current_scope = if *TRAINING_CLUSTER_SCOPE.read() {
        DataScope::Cluster
    } else {
        DataScope::Local
    };
    let scan_pending = cluster_scan.pending();
    let selected_step = use_signal(|| None::<SelectedStep>);

    rsx! {
        PageContainer {
            PageTitle {
                title: "Training".to_string(),
                subtitle: None,
                icon: Some(&icondata::AiRadarChartOutlined),
                header_right: if show_controls {
                    Some(rsx! {
                        if current_scope == DataScope::Local {
                            PollStatusBar {
                                interval_secs: POLL_MS / 1000,
                                poll_tick: local_tick,
                            }
                        }
                        RefreshButton {
                            onclick: move |_| *TRAINING_REFRESH.write() += 1,
                        }
                    })
                } else {
                    None
                },
            }

            if show_controls {
                TrainingScopeBar {
                    scope: current_scope,
                    peer_count,
                    scan_pending,
                    on_local: move |_| *TRAINING_CLUSTER_SCOPE.write() = false,
                    on_cluster_scan: move |_| {
                        *TRAINING_CLUSTER_SCOPE.write() = true;
                        *TRAINING_REFRESH.write() += 1;
                    },
                }
            }

            if current_scope == DataScope::Cluster {
                if let Some(Ok(output)) = cluster_scan.value() {
                    {cluster_nodes_failed_banner(&output().nodes_failed)}
                }
            }

            div { class: "space-y-4",
                div { class: "min-w-0",
                    if current_scope == DataScope::Local {
                        AsyncBoundary {
                            message: Some("Loading step timings…".to_string()),
                            LocalStepMatrixPanel {
                                refresh_tick: local_tick,
                                selected_step,
                            }
                        }
                    } else if scan_pending {
                        Card {
                            title: STEP_CARD_TITLE,
                            LoadingState { message: Some("Scanning cluster…".to_string()) }
                        }
                    } else if let Some(Err(err)) = cluster_scan.value() {
                        Card {
                            title: STEP_CARD_TITLE,
                            AppErrorDisplay {
                                error: AppError::Api(err.to_string()),
                                title: Some("Cluster scan failed".to_string()),
                            }
                        }
                    } else if let Some(Ok(output)) = cluster_scan.value() {
                        ClusterStepMatrixPanel {
                            matrix: output().matrix.clone(),
                            selected_step,
                        }
                    } else {
                        Card {
                            title: STEP_CARD_TITLE,
                            EmptyState { message: "Run a cluster scan to compare ranks.".to_string() }
                        }
                    }
                }
                TrainingPlacement { node_state: node_state.clone() }
            }

            StepInspectorOverlay { selected: selected_step }

            div { class: "mt-4 space-y-3",
                if current_scope == DataScope::Local {
                    AsyncBoundary {
                        message: Some("Loading module data…".to_string()),
                        LocalModuleHotspotsPanel { refresh_tick: local_tick }
                    }
                    AsyncBoundary {
                        message: Some("Loading collective data…".to_string()),
                        LocalCommPanel { refresh_tick: local_tick }
                    }
                } else if let Some(Ok(output)) = cluster_scan.value() {
                    {render_comm_cluster_collapsible(&output().comm)}
                } else if scan_pending {
                    CollapsibleCommPlaceholder {}
                }
            }
        }
    }
}

#[component]
fn TrainingPlacement(node_state: Option<Result<Vec<Node>, AppError>>) -> Element {
    match node_state {
        Some(Ok(nodes)) if !nodes.is_empty() => rsx! {
            Card {
                title: "Placement",
                content_class: Some("p-4"),
                PlacementDiagram { placement: build_placement(&nodes) }
            }
        },
        _ => rsx! {},
    }
}

#[component]
fn PlacementDiagram(placement: PlacementModel) -> Element {
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
                    span { class: "text-gray-400", "parallel roles unavailable" }
                }
                if missing_ranks > 0 {
                    span { class: "rounded-full bg-amber-100 px-2 py-0.5 font-medium text-amber-800",
                        "{missing_ranks} missing"
                    }
                }
            }

            PlacementOverview { placement: placement.clone() }
        }
    }
}

#[component]
fn PlacementOverview(placement: PlacementModel) -> Element {
    let mut hovered_rank = use_signal(|| None::<i32>);
    let active_process = hovered_rank().and_then(|rank| {
        placement
            .hosts
            .iter()
            .flat_map(|host| host.processes.iter())
            .find(|process| process.rank == Some(rank))
            .cloned()
    });
    let group_sizes = active_process
        .as_ref()
        .and_then(|active| placement_group_sizes(&placement, active));
    let host_columns = placement_host_columns(placement.hosts.len());

    rsx! {
        div {
            class: "rounded-md border border-gray-200 bg-gray-50 px-3 py-2.5",
            onmouseleave: move |_| hovered_rank.set(None),
            div { class: "mb-2 flex flex-wrap items-center justify-between gap-2",
                div { class: "flex items-center gap-2",
                    span { class: "text-[10px] font-medium uppercase tracking-wide text-gray-500", "Overview" }
                    if let Some(rank) = hovered_rank() {
                        span { class: "font-mono text-[10px] font-semibold text-blue-700", "R{rank}" }
                    }
                }
                div { class: "flex items-center gap-3 text-[9px] text-gray-500",
                    PlacementGroupLegend {
                        label: "TP",
                        count: group_sizes.map(|sizes| sizes.tensor),
                        class: "border-violet-500 bg-violet-100",
                    }
                    PlacementGroupLegend {
                        label: "DP",
                        count: group_sizes.map(|sizes| sizes.data),
                        class: "border-emerald-500 bg-emerald-100",
                    }
                    PlacementGroupLegend {
                        label: "PP",
                        count: group_sizes.map(|sizes| sizes.pipeline),
                        class: "border-amber-500 bg-amber-100",
                    }
                    span { class: "text-gray-400", "hover a GPU" }
                }
            }
            div { class: "overflow-x-auto pb-0.5",
                div {
                    class: "grid min-w-max gap-2",
                    style: "grid-template-columns: repeat({host_columns}, 26px);",
                    for (host_index, host) in placement.hosts.iter().enumerate() {
                        div {
                            class: "rounded border border-gray-200 bg-white p-1",
                            title: "{host.name}",
                            div { class: "mb-1 truncate text-center font-mono text-[8px] text-gray-400", "H{host_index}" }
                            div { class: "grid grid-cols-1 justify-items-center gap-0.5",
                                for process in host.processes.iter() {
                                    {
                                        let rank = process.rank;
                                        let rank_label = rank.map(|value| format!("R{value}")).unwrap_or_else(|| "R?".to_string());
                                        let local_rank = process.local_rank.map(|value| value.to_string()).unwrap_or_else(|| "?".to_string());
                                        let gpu_label = format!("GPU{local_rank}");
                                        let status = process.status.as_deref().unwrap_or("unknown");
                                        let role = process.role_label.as_deref().unwrap_or("rank");
                                        let coordinates = process.coordinates.iter()
                                            .map(|(dimension, value)| format!("{}{}", dimension.chars().next().unwrap_or('?').to_ascii_uppercase(), value))
                                            .collect::<Vec<_>>()
                                            .join(" ");
                                        let group = active_process
                                            .as_ref()
                                            .and_then(|active| placement_group_membership(active, process));
                                        let cell_class = placement_overview_cell_class(group, process.status.as_deref());
                                        let cell_title = if coordinates.is_empty() {
                                            format!("{rank_label} · {} · {gpu_label} · {status} · {role}", host.name)
                                        } else {
                                            format!("{rank_label} · {} · {gpu_label} · {status} · {role} · {coordinates}", host.name)
                                        };
                                        rsx! {
                                            button {
                                                r#type: "button",
                                                class: "flex h-4 w-4 items-center justify-center rounded-[2px] border text-[7px] font-mono transition-colors {cell_class}",
                                                aria_label: "{cell_title}",
                                                title: "{cell_title}",
                                                onmouseover: move |_| hovered_rank.set(rank),
                                                onfocus: move |_| hovered_rank.set(rank),
                                                onclick: move |_| hovered_rank.set(rank),
                                                onblur: move |_| hovered_rank.set(None),
                                                "{local_rank}"
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
}

#[component]
fn PlacementGroupLegend(label: &'static str, count: Option<usize>, class: &'static str) -> Element {
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

fn placement_coordinate<'a>(process: &'a PlacementProcess, dimension: &str) -> Option<&'a str> {
    process
        .coordinates
        .iter()
        .find_map(|(key, value)| (key == dimension).then_some(value.as_str()))
}

fn placement_group_membership(
    active: &PlacementProcess,
    candidate: &PlacementProcess,
) -> Option<PlacementGroup> {
    if active.rank == candidate.rank {
        return Some(PlacementGroup::Focus);
    }

    let active_dp = placement_coordinate(active, "dp")?;
    let active_pp = placement_coordinate(active, "pp")?;
    let active_tp = placement_coordinate(active, "tp")?;
    let candidate_dp = placement_coordinate(candidate, "dp")?;
    let candidate_pp = placement_coordinate(candidate, "pp")?;
    let candidate_tp = placement_coordinate(candidate, "tp")?;

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
    placement_coordinate(active, "dp")?;
    placement_coordinate(active, "pp")?;
    placement_coordinate(active, "tp")?;

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

fn placement_overview_cell_class(
    group: Option<PlacementGroup>,
    status: Option<&str>,
) -> &'static str {
    match group {
        Some(PlacementGroup::Focus) => {
            "border-blue-700 bg-blue-600 text-white ring-2 ring-blue-200"
        }
        Some(PlacementGroup::Tensor) => {
            "border-dashed border-violet-500 bg-violet-100 text-violet-900"
        }
        Some(PlacementGroup::Data) => {
            "border-dashed border-emerald-500 bg-emerald-100 text-emerald-900"
        }
        Some(PlacementGroup::Pipeline) => {
            "border-dashed border-amber-500 bg-amber-100 text-amber-900"
        }
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

fn placement_host_columns(host_count: usize) -> usize {
    host_count.clamp(1, 8)
}

fn parse_parallel_coordinates(role: Option<&str>) -> Vec<(String, String)> {
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

fn build_placement(nodes: &[Node]) -> PlacementModel {
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
        let coordinates = parse_parallel_coordinates(node.role.as_deref());
        let role_label = node
            .role_name
            .clone()
            .filter(|role| !role.trim().is_empty())
            .or_else(|| coordinates.is_empty().then(|| node.role.clone()).flatten());
        let host = if node.host.trim().is_empty() {
            "Unknown host".to_string()
        } else {
            node.host.clone()
        };
        hosts.entry(host).or_default().push(PlacementProcess {
            rank: node.rank,
            local_rank: node.local_rank,
            role_label,
            coordinates,
            status: node.status.clone(),
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
    hosts.sort_by(|a, b| a.name.cmp(&b.name));

    let observed_ranks = ranks.len();
    if expected_ranks == 0 {
        expected_ranks = observed_ranks;
    }
    let mut coordinate_values: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for process in hosts.iter().flat_map(|host| host.processes.iter()) {
        for (dimension, value) in &process.coordinates {
            coordinate_values
                .entry(dimension.clone())
                .or_default()
                .insert(value.clone());
        }
    }
    let parallel_sizes = ["dp", "pp", "tp", "sp", "cp", "ep"]
        .into_iter()
        .filter_map(|dimension| {
            coordinate_values
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

fn queue_investigate_skill(skill_id: String) {
    if load_skill(&skill_id).is_none() {
        return;
    }
    *AGENT_PANEL_OPEN.write() = true;
    *AGENT_INPUT.write() = format!("/{skill_id}");
}

#[component]
fn TrainingScopeBar(
    scope: DataScope,
    peer_count: usize,
    scan_pending: bool,
    on_local: EventHandler<()>,
    on_cluster_scan: EventHandler<()>,
) -> Element {
    let refresh_secs = POLL_MS / 1000;
    let cluster_title: &'static str = if scan_pending {
        "Scan in progress…"
    } else {
        "One-shot fan-out across training nodes"
    };
    let status = if scope == DataScope::Local {
        format!("Live · refreshes every {refresh_secs}s")
    } else if peer_count == 0 {
        "No peer nodes detected".to_string()
    } else {
        format!("On-demand scan · {peer_count} peer(s)")
    };
    rsx! {
        div { class: "flex flex-wrap items-center justify-between gap-3 mb-4",
            div { class: "inline-flex items-center gap-0.5 p-0.5 rounded-lg bg-gray-100 border border-gray-200",
                WidthSegment {
                    label: "This node",
                    selected: scope == DataScope::Local,
                    title: "Auto-refresh local train.step spans",
                    onclick: move |_| on_local.call(()),
                }
                WidthSegment {
                    label: "Cluster",
                    selected: scope == DataScope::Cluster,
                    title: cluster_title,
                    onclick: move |_| on_cluster_scan.call(()),
                }
            }
            p { class: "text-xs text-gray-500", "{status}" }
        }
    }
}

#[component]
fn StepInspectorOverlay(selected: Signal<Option<SelectedStep>>) -> Element {
    let Some(sel) = selected() else {
        return rsx! {};
    };

    rsx! {
        div {
            class: "fixed inset-0 z-40 flex justify-end pointer-events-none",
            div {
                class: "absolute inset-0 bg-black/20 pointer-events-auto",
                onclick: move |_| selected.set(None),
            }
            div {
                class: "relative h-full w-full max-w-md flex flex-col pointer-events-auto \
                         bg-white shadow-2xl border-l border-gray-200",
                role: "dialog",
                aria_label: "Step inspector",
                onclick: move |e| e.stop_propagation(),
                div {
                    class: "shrink-0 px-4 py-3 border-b border-gray-100 bg-gradient-to-r from-violet-50/80 to-white \
                             flex items-center justify-between gap-3",
                    div { class: "flex items-center gap-2 min-w-0",
                        Icon { icon: &icondata::AiSearchOutlined, class: "w-4 h-4 text-violet-600 shrink-0" }
                        div { class: "min-w-0",
                            div { class: "text-sm font-semibold text-gray-900", "Step inspector" }
                            div { class: "text-[10px] text-gray-500 truncate",
                                "Step {sel.display_step} · rank {sel.rank}"
                            }
                        }
                    }
                    button {
                        class: "shrink-0 p-1.5 rounded-md text-gray-400 hover:text-gray-700 hover:bg-gray-100 transition-colors",
                        title: "Close",
                        aria_label: "Close step inspector",
                        onclick: move |_| selected.set(None),
                        Icon { icon: &icondata::AiCloseOutlined, class: "w-4 h-4" }
                    }
                }
                div { class: "flex-1 min-h-0 overflow-y-auto p-4 space-y-3",
                    StepDetailLoaded { sel }
                }
            }
        }
    }
}

#[component]
fn StepDetailLoaded(sel: SelectedStep) -> Element {
    let display_step = sel.display_step;
    let coord_step = sel.coord_step;
    let spans = use_app_resource(move || {
        let d = display_step;
        async move { ApiClient::new().execute_query(&step_span_sql(d)).await }
    });
    let modules = use_app_resource(move || {
        let c = coord_step;
        async move { ApiClient::new().execute_query(&step_module_sql(c)).await }
    });

    let spans_res = spans.suspend()?();
    let modules_res = modules.suspend()?();
    let avg_hint = format_step_ms(sel.duration_ms);

    rsx! {
        div { class: "rounded-lg border border-violet-100 bg-violet-50/50 px-3 py-2",
            div { class: "flex items-baseline justify-between gap-2",
                span { class: "text-xs text-gray-500", "Step" }
                span { class: "text-lg font-semibold font-mono text-gray-900", "{sel.display_step}" }
            }
            div { class: "mt-1 text-2xl font-semibold font-mono text-violet-800", "{avg_hint}" }
            div { class: "mt-1 text-[10px] font-mono text-gray-500",
                "rank {sel.rank} · trace step {sel.coord_step}"
            }
        }
        div { class: "flex flex-wrap gap-1.5",
            for (id, label) in QUICK_SKILLS {
                ChipButton {
                    label: (*label).to_string(),
                    disabled: ui_agent_busy(),
                    onclick: {
                        let skill_id = (*id).to_string();
                        move |_| queue_investigate_skill(skill_id.clone())
                    },
                }
            }
        }
        p { class: "text-[10px] text-gray-400", "Opens Investigate with skill · context pinned" }
        StepDetailSection {
            title: "Span breakdown",
            hint: "Nested spans in this train.step (forward / backward / optim)",
            result: spans_res,
        }
        StepDetailSection {
            title: "Module hooks",
            hint: "TorchProbe samples for this step (requires PROBING_TORCH_PROFILING=on)",
            result: modules_res,
        }
    }
}

#[component]
fn StepDetailSection(
    title: &'static str,
    hint: &'static str,
    result: Result<probing_proto::prelude::DataFrame, AppError>,
) -> Element {
    rsx! {
        div { class: "space-y-1",
            p { class: "text-xs font-medium text-gray-700", "{title}" }
            p { class: "text-[10px] text-gray-400", "{hint}" }
            match result {
                Ok(ref data) if dataframe_rows(data) == 0 => rsx! {
                    p { class: "text-[10px] text-gray-400 italic py-2", "No data for this step." }
                },
                Ok(ref data) => rsx! {
                    div { class: "overflow-x-auto border border-gray-200 rounded-md max-h-40",
                        DataFrameView { df: data.clone() }
                    }
                },
                Err(ref e) => rsx! {
                    p { class: "text-[10px] text-red-600", "{e}" }
                },
            }
        }
    }
}

#[component]
fn LocalStepMatrixPanel(refresh_tick: u32, selected_step: Signal<Option<SelectedStep>>) -> Element {
    let matrix = use_app_resource(move || {
        let _ = refresh_tick;
        async move { ApiClient::new().fetch_step_matrix(STEP_LIMIT, false).await }
    });
    render_step_matrix_result(&matrix.suspend()?(), selected_step)
}

#[component]
fn ClusterStepMatrixPanel(
    matrix: Result<StepMatrixResponse, AppError>,
    selected_step: Signal<Option<SelectedStep>>,
) -> Element {
    render_step_matrix_result(&matrix, selected_step)
}

#[component]
fn LocalModuleHotspotsPanel(refresh_tick: u32) -> Element {
    let modules = use_app_resource(move || {
        let _ = refresh_tick;
        async move { ApiClient::new().execute_query(MODULE_HOTSPOTS_SQL).await }
    });
    let phases = use_app_resource(move || {
        let _ = refresh_tick;
        async move { ApiClient::new().execute_query(STEP_PHASE_SQL).await }
    });

    let modules_res = modules.suspend()?();
    let phases_res = phases.suspend()?();

    render_module_hotspots(&modules_res, &phases_res)
}

#[component]
fn LocalCommPanel(refresh_tick: u32) -> Element {
    let comm = use_app_resource(move || {
        let _ = refresh_tick;
        async move {
            ApiClient::new()
                .execute_query(&format!("{COMM_SQL}{COMM_LIMIT}"))
                .await
        }
    });
    let summary = use_app_resource(move || {
        let _ = refresh_tick;
        async move { ApiClient::new().execute_query(COMM_SUMMARY_SQL).await }
    });

    let comm_res = comm.suspend()?();
    let summary_res = summary.suspend()?();
    render_comm_local_result(&comm_res, &summary_res)
}

#[component]
fn CollapsibleCommPlaceholder() -> Element {
    rsx! {
        CollapsibleCardWithIcon {
            title: "Collective Communications".to_string(),
            icon: rsx! {
                Icon { icon: &icondata::AiClusterOutlined, class: "w-4 h-4 text-gray-500" }
            },
            children: rsx! {
                LoadingState { message: Some("Scanning cluster…".to_string()) }
            },
        }
    }
}

fn render_comm_cluster_collapsible(result: &Result<ClusterQueryResponse, AppError>) -> Element {
    match result {
        Ok(resp) if dataframe_rows(&resp.dataframe) > 0 => {
            let mut note = format!("cluster scan · {} nodes queried", resp.meta.nodes_queried);
            if !resp.meta.nodes_failed.is_empty() {
                note.push_str(&format!(" · {} failed", resp.meta.nodes_failed.len()));
            }
            let df = resp.dataframe.clone();
            let rows = dataframe_rows(&df);
            let df_for_click = df.clone();
            rsx! {
                CollapsibleCardWithIcon {
                    title: "Collective Communications".to_string(),
                    badge: Some(format!("{rows} rows")),
                    accent_border: Some("border-l-violet-400".to_string()),
                    icon: rsx! {
                        Icon { icon: &icondata::AiClusterOutlined, class: "w-4 h-4 text-violet-600" }
                    },
                    children: rsx! {
                        div { class: "space-y-3",
                            p { class: "text-xs text-gray-500", "{note}" }
                            p { class: "text-[10px] text-gray-500",
                                "Click a row to set investigation context (rank / op columns)."
                            }
                            div { class: "overflow-x-auto border border-gray-200 rounded-lg max-h-96",
                                DataFrameView {
                                    df: df.clone(),
                                    on_row_click: EventHandler::new(move |row: usize| {
                                        apply_context_from_dataframe_row(&df_for_click, row);
                                    }),
                                }
                            }
                            p { class: "text-xs text-gray-400",
                                "Lite mode: timing + context in python.comm_collective · full spans: SET probing.torch.collective.mode=full"
                            }
                        }
                    },
                }
            }
        }
        Ok(_) => rsx! {
            CollapsibleCardWithIcon {
                title: "Collective Communications".to_string(),
                icon: rsx! {
                    Icon { icon: &icondata::AiClusterOutlined, class: "w-4 h-4 text-gray-500" }
                },
                children: rsx! {
                    EmptyState { message: "No collective rows returned from cluster scan.".to_string() }
                },
            }
        },
        Err(err) => rsx! {
            CollapsibleCardWithIcon {
                title: "Collective Communications".to_string(),
                icon: rsx! {
                    Icon { icon: &icondata::AiClusterOutlined, class: "w-4 h-4 text-gray-500" }
                },
                children: rsx! {
                    AppErrorDisplay { error: err.clone(), title: None }
                },
            }
        },
    }
}

fn cluster_nodes_failed_banner(nodes: &[String]) -> Element {
    if nodes.is_empty() {
        return rsx! { div {} };
    }
    rsx! {
        div {
            class: "mb-4 rounded-lg border border-amber-300 bg-amber-50 px-4 py-3 text-sm text-amber-950",
            p { class: "font-medium",
                "Partial cluster scan — {nodes.len()} node(s) did not respond"
            }
            p { class: "mt-1 text-xs text-amber-800",
                "Results below may be incomplete. Check that peers are running and reachable."
            }
            ul { class: "mt-2 text-xs font-mono text-amber-900 list-disc pl-5 space-y-0.5",
                for addr in nodes.iter() {
                    li { "{addr}" }
                }
            }
        }
    }
}

fn step_summary_stats(
    samples: &[StepDurationSample],
    single_rank: bool,
) -> Vec<(String, String, Option<String>)> {
    if samples.is_empty() {
        return Vec::new();
    }
    if single_rank {
        return single_rank_summary_stats(&build_step_series(samples));
    }
    let rank_count = samples.iter().map(|s| s.rank).collect::<HashSet<_>>().len();
    let step_count = samples
        .iter()
        .map(|s| s.local_step)
        .collect::<HashSet<_>>()
        .len();
    let max_ms = samples.iter().map(|s| s.duration_ms).fold(0.0f64, f64::max);
    let outliers = count_outlier_cells(samples);
    vec![
        ("Ranks".to_string(), rank_count.to_string(), None),
        ("Steps".to_string(), step_count.to_string(), None),
        (
            "Max step".to_string(),
            if max_ms > 0.0 {
                format!("{max_ms:.0} ms")
            } else {
                "—".to_string()
            },
            None,
        ),
        ("Outliers".to_string(), outliers.to_string(), None),
    ]
}

fn build_step_series(samples: &[StepDurationSample]) -> Vec<(i64, f64)> {
    build_step_points(samples)
        .into_iter()
        .map(|p| (p.display_step, p.duration_ms))
        .collect()
}

fn build_step_points(samples: &[StepDurationSample]) -> Vec<StepPoint> {
    let rank = primary_rank(samples);
    let mut points: Vec<StepPoint> = samples
        .iter()
        .filter(|s| display_rank(s.rank) == rank && s.local_step >= 0)
        .map(|s| StepPoint {
            display_step: s.local_step,
            coord_step: trace_step(s.coord_step, s.local_step),
            duration_ms: s.duration_ms,
        })
        .collect();
    points.sort_by_key(|p| p.display_step);
    if points.len() > STEP_LIMIT {
        points = points[points.len().saturating_sub(STEP_LIMIT)..].to_vec();
    }
    points
}

fn build_coord_lookup(samples: &[StepDurationSample]) -> HashMap<(i32, i64), (i64, f64)> {
    let mut lookup = HashMap::new();
    for s in samples {
        if s.local_step < 0 {
            continue;
        }
        let rank = display_rank(s.rank);
        lookup
            .entry((rank, s.local_step))
            .and_modify(|(_, dur)| {
                if s.duration_ms > *dur {
                    *dur = s.duration_ms;
                }
            })
            .or_insert((trace_step(s.coord_step, s.local_step), s.duration_ms));
    }
    lookup
}

fn percentile(values: &[f64], p: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut v = values.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let idx = ((v.len() as f64 - 1.0) * p).round() as usize;
    v.get(idx).copied().unwrap_or(0.0)
}

fn format_step_ms(ms: f64) -> String {
    if ms >= 100.0 {
        format!("{ms:.0} ms")
    } else if ms >= 1.0 {
        format!("{ms:.1} ms")
    } else if ms > 0.0 {
        format!("{ms:.2} ms")
    } else {
        "0 ms".to_string()
    }
}

fn single_rank_summary_stats(series: &[(i64, f64)]) -> Vec<(String, String, Option<String>)> {
    if series.is_empty() {
        return Vec::new();
    }
    let durations: Vec<f64> = series.iter().map(|(_, d)| *d).collect();
    let avg = durations.iter().sum::<f64>() / durations.len() as f64;
    let max = durations.iter().copied().fold(0.0f64, f64::max);
    let p95 = percentile(&durations, 0.95);
    let (latest_step, latest_ms) = series.last().copied().unwrap_or((-1, 0.0));

    vec![
        (
            "Latest".to_string(),
            format_step_ms(latest_ms),
            Some(format!("step {latest_step}")),
        ),
        ("Average".to_string(), format_step_ms(avg), None),
        ("P95".to_string(), format_step_ms(p95), None),
        ("Maximum".to_string(), format_step_ms(max), None),
    ]
}

/// Single-process spans may carry ``rank: -1`` when ``RANK`` was unset; treat as 0.
fn display_rank(rank: i32) -> i32 {
    if rank < 0 {
        0
    } else {
        rank
    }
}

fn primary_rank(samples: &[StepDurationSample]) -> i32 {
    samples
        .iter()
        .map(|s| display_rank(s.rank))
        .next()
        .unwrap_or(0)
}

fn count_outlier_cells(samples: &[StepDurationSample]) -> usize {
    let (_, _, cells, _) = build_heatmap(samples);
    cells.values().filter(|c| c.outlier).count()
}

fn render_step_matrix_result(
    result: &Result<StepMatrixResponse, AppError>,
    selected_step: Signal<Option<SelectedStep>>,
) -> Element {
    match result {
        Ok(resp) if resp.samples.is_empty() => rsx! {
            Card {
                title: STEP_CARD_TITLE,
                EmptyState {
                    message: "No train.step spans yet. Enable phase hooks with probing.attach_training_phases(model, optimizer) or record train.step spans manually.".to_string()
                }
            }
        },
        Ok(resp) => {
            let (ranks, steps, cells, max_ms) = build_heatmap(&resp.samples);
            let single_rank = ranks.len() <= 1;
            let coord_lookup = build_coord_lookup(&resp.samples);
            let scope_note = if resp.cluster {
                let mut note = format!("cluster scan · {} nodes queried", resp.nodes_queried);
                if !resp.nodes_failed.is_empty() {
                    note.push_str(&format!(" · {} failed", resp.nodes_failed.len()));
                }
                note
            } else {
                "local node · auto-refresh".to_string()
            };
            let stats = step_summary_stats(&resp.samples, single_rank);
            let rank = primary_rank(&resp.samples);
            rsx! {
                Card {
                    title: STEP_CARD_TITLE,
                    content_class: Some("p-4"),
                    div { class: "space-y-3",
                        div { class: "grid grid-cols-4 divide-x divide-gray-200",
                            for (label, value, hint) in stats {
                                div { class: "min-w-0 px-4 first:pl-0 last:pr-0",
                                    p { class: "text-[10px] font-medium uppercase tracking-wide text-gray-500", "{label}" }
                                    p { class: "mt-1 truncate text-xl font-semibold text-gray-900", "{value}" }
                                    if let Some(hint) = hint {
                                        p { class: "mt-0.5 text-[10px] text-gray-400", "{hint}" }
                                    }
                                }
                            }
                        }
                        if single_rank {
                            StepDurationTimeline {
                                rank,
                                points: build_step_points(&resp.samples),
                                selected_step,
                            }
                        } else {
                            p { class: "text-[10px] text-gray-500",
                                "{scope_note} · darker cells are slower; outlined cells exceed 1.2× the step median"
                            }
                            StepHeatmap {
                                ranks: ranks.clone(),
                                steps: steps.clone(),
                                cells: cells.clone(),
                                max_ms,
                                coord_lookup: coord_lookup.clone(),
                                selected_step,
                            }
                        }
                    }
                }
            }
        }
        Err(err) => rsx! {
            Card {
                title: STEP_CARD_TITLE,
                AppErrorDisplay { error: err.clone(), title: None }
            }
        },
    }
}

fn render_comm_local_result(
    result: &Result<probing_proto::prelude::DataFrame, AppError>,
    summary: &Result<probing_proto::prelude::DataFrame, AppError>,
) -> Element {
    match result {
        Ok(df) if df.cols.is_empty() || dataframe_rows(df) == 0 => rsx! {
            CollapsibleCardWithIcon {
                title: "Collective Communications".to_string(),
                icon: rsx! {
                    Icon { icon: &icondata::AiClusterOutlined, class: "w-4 h-4 text-gray-500" }
                },
                children: rsx! {
                    EmptyState {
                        message: "No collective samples on this node. Enable with PROBING_TORCH_COLLECTIVE_ENABLE=1 or SET probing.torch.collective.enable=1.".to_string()
                    }
                },
            }
        },
        Ok(df) => {
            let summary_df = summary.as_ref().ok().filter(|s| dataframe_rows(s) > 0);
            comm_table_collapsible(df, summary_df, "local node · auto-refresh")
        }
        Err(err) => rsx! {
            CollapsibleCardWithIcon {
                title: "Collective Communications".to_string(),
                icon: rsx! {
                    Icon { icon: &icondata::AiClusterOutlined, class: "w-4 h-4 text-gray-500" }
                },
                children: rsx! {
                    AppErrorDisplay { error: err.clone(), title: None }
                },
            }
        },
    }
}

fn dataframe_rows(df: &probing_proto::prelude::DataFrame) -> usize {
    df.row_count()
}

type HeatmapData = (Vec<i32>, Vec<i64>, HashMap<(i32, i64), HeatCell>, f64);

fn build_heatmap(samples: &[StepDurationSample]) -> HeatmapData {
    let mut rank_set = HashSet::new();
    let mut step_set = HashSet::new();
    let mut raw: HashMap<(i32, i64), f64> = HashMap::new();

    for s in samples {
        if s.local_step < 0 {
            continue;
        }
        let rank = display_rank(s.rank);
        rank_set.insert(rank);
        step_set.insert(s.local_step);
        raw.entry((rank, s.local_step))
            .and_modify(|v| *v = v.max(s.duration_ms))
            .or_insert(s.duration_ms);
    }

    let mut ranks: Vec<i32> = rank_set.into_iter().collect();
    ranks.sort();
    let mut steps: Vec<i64> = step_set.into_iter().collect();
    steps.sort();
    if steps.len() > 40 {
        steps = steps[steps.len().saturating_sub(40)..].to_vec();
    }

    let mut step_medians: HashMap<i64, f64> = HashMap::new();
    for step in &steps {
        let mut vals: Vec<f64> = ranks
            .iter()
            .filter_map(|r| raw.get(&(*r, *step)).copied())
            .collect();
        if vals.is_empty() {
            continue;
        }
        vals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let mid = vals[vals.len() / 2];
        step_medians.insert(*step, mid);
    }

    let max_ms = raw.values().copied().fold(0.0f64, f64::max).max(1.0);
    let mut cells = HashMap::new();
    for ((rank, step), dur) in raw {
        if !steps.contains(&step) {
            continue;
        }
        let median = step_medians.get(&step).copied().unwrap_or(dur);
        let outlier = dur > median * 1.2 && ranks.len() > 1;
        cells.insert(
            (rank, step),
            HeatCell {
                duration_ms: dur,
                outlier,
            },
        );
    }

    (ranks, steps, cells, max_ms)
}

#[component]
fn StepHeatmap(
    ranks: Vec<i32>,
    steps: Vec<i64>,
    cells: HashMap<(i32, i64), HeatCell>,
    max_ms: f64,
    coord_lookup: HashMap<(i32, i64), (i64, f64)>,
    selected_step: Signal<Option<SelectedStep>>,
) -> Element {
    let featured = ranks.len() <= 1;
    let cell_min = if featured {
        "min-w-[48px]"
    } else {
        "min-w-[28px]"
    };
    let cell_h = if featured { "h-10" } else { "h-7" };

    rsx! {
        div { class: "overflow-x-auto",
            div {
                class: "inline-grid gap-1",
                style: "grid-template-columns: auto repeat({steps.len()}, minmax(0, 1fr));",
                div { class: "text-xs text-gray-400 pr-2 self-end pb-1", "rank \\ step" }
                for step in steps.iter() {
                    div {
                        class: "text-[10px] text-gray-500 text-center pb-1 font-mono",
                        "{step}"
                    }
                }
                for rank in ranks.iter() {
                    div {
                        class: "text-xs font-mono text-gray-600 pr-2 flex items-center justify-end",
                        "R{rank}"
                    }
                    for step in steps.iter() {
                        {
                            let cell = cells.get(&(*rank, *step));
                            let (bg, title, ring) = if let Some(c) = cell {
                                let pct = (c.duration_ms / max_ms).clamp(0.0, 1.0);
                                let alpha = 0.15 + pct * 0.85;
                                let ring = if c.outlier {
                                    "ring-2 ring-red-500 ring-offset-1"
                                } else {
                                    ""
                                };
                                (
                                    format!("background-color: rgba(109, 40, 217, {alpha});"),
                                    format!("rank {rank} step {step}: {:.1} ms — click to set context", c.duration_ms),
                                    ring.to_string(),
                                )
                            } else {
                                (
                                    "background-color: rgb(243 244 246);".to_string(),
                                    format!("rank {rank} step {step}: no data"),
                                    String::new(),
                                )
                            };
                            let rank_val = *rank;
                            let step_val = *step;
                            let clickable = cell.is_some();
                            let is_selected = selected_step()
                                .map(|s| s.rank == rank_val && s.display_step == step_val)
                                .unwrap_or(false);
                            let selected_ring = if is_selected {
                                "ring-2 ring-blue-500 ring-offset-1"
                            } else {
                                ""
                            };
                            let (coord_step, duration_ms) = coord_lookup
                                .get(&(rank_val, step_val))
                                .copied()
                                .unwrap_or((step_val, cell.map(|c| c.duration_ms).unwrap_or(0.0)));
                            rsx! {
                                button {
                                    r#type: "button",
                                    disabled: !clickable,
                                    class: "rounded-sm {cell_min} {cell_h} {ring} {selected_ring} disabled:cursor-default",
                                    class: if clickable { "cursor-pointer hover:ring-2 hover:ring-blue-300 hover:ring-offset-1" } else { "" },
                                    style: "{bg}",
                                    title: "{title}",
                                    onclick: move |_| {
                                        if clickable {
                                            select_training_step(
                                                rank_val,
                                                step_val,
                                                coord_step,
                                                duration_ms,
                                                selected_step,
                                            );
                                        }
                                    },
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
fn StepDurationTimeline(
    rank: i32,
    points: Vec<StepPoint>,
    selected_step: Signal<Option<SelectedStep>>,
) -> Element {
    if points.is_empty() {
        return rsx! {
            p { class: "text-xs text-gray-400 italic py-4 text-center",
                "Step samples returned but none matched this rank — try refreshing or check train.step span attributes."
            }
        };
    }

    let width = 1000.0;
    let height = 170.0;
    let pad_left = 46.0;
    let pad_right = 14.0;
    let pad_top = 12.0;
    let pad_bottom = 28.0;
    let plot_width = width - pad_left - pad_right;
    let plot_height = height - pad_top - pad_bottom;
    let min_ms = points
        .iter()
        .map(|point| point.duration_ms)
        .fold(f64::INFINITY, f64::min);
    let max_ms = points
        .iter()
        .map(|point| point.duration_ms)
        .fold(f64::NEG_INFINITY, f64::max);
    let y_padding = ((max_ms - min_ms) * 0.08).max(max_ms.abs() * 0.02).max(0.1);
    let y_min = (min_ms - y_padding).max(0.0);
    let y_max = max_ms + y_padding;
    let y_span = (y_max - y_min).max(0.1);
    let avg_ms = points.iter().map(|p| p.duration_ms).sum::<f64>() / points.len() as f64;
    let latest_idx = points.len().saturating_sub(1);
    let point_count_span = latest_idx.max(1) as f64;
    let chart_points = points
        .iter()
        .enumerate()
        .map(|(index, point)| {
            let x = pad_left + index as f64 / point_count_span * plot_width;
            let y = pad_top + (y_max - point.duration_ms) / y_span * plot_height;
            (point.clone(), x, y)
        })
        .collect::<Vec<_>>();
    let line_points = chart_points
        .iter()
        .map(|(_, x, y)| format!("{x:.1},{y:.1}"))
        .collect::<Vec<_>>()
        .join(" ");
    let avg_y = pad_top + (y_max - avg_ms) / y_span * plot_height;
    let first_step = points
        .first()
        .map(|point| point.display_step)
        .unwrap_or_default();
    let last_step = points
        .last()
        .map(|point| point.display_step)
        .unwrap_or_default();
    rsx! {
        div { class: "border-t border-gray-100 pt-3",
            div { class: "mb-1 flex items-center justify-between gap-2 text-[10px] text-gray-500",
                span { "Recent steps" }
                span { class: "font-mono", "rank {rank}" }
            }
            svg {
                class: "h-44 w-full",
                view_box: "0 0 {width} {height}",
                preserve_aspect_ratio: "none",
                for tick in 0..=3 {
                    {
                        let ratio = tick as f64 / 3.0;
                        let y = pad_top + ratio * plot_height;
                        let tick_value = y_max - ratio * y_span;
                        rsx! {
                            line {
                                x1: "{pad_left}", y1: "{y}",
                                x2: "{pad_left + plot_width}", y2: "{y}",
                                stroke: "#e5e7eb", stroke_width: "1",
                            }
                            text {
                                x: "{pad_left - 6.0}", y: "{y + 3.0}",
                                text_anchor: "end", font_size: "9", fill: "#9ca3af",
                                "{format_step_ms(tick_value)}"
                            }
                        }
                    }
                }
                line {
                    x1: "{pad_left}", y1: "{avg_y}",
                    x2: "{pad_left + plot_width}", y2: "{avg_y}",
                    stroke: "#9ca3af", stroke_width: "1", stroke_dasharray: "4 4",
                }
                polyline {
                    points: "{line_points}",
                    fill: "none",
                    stroke: "#2563eb",
                    stroke_width: "2.5",
                    stroke_linejoin: "round",
                    stroke_linecap: "round",
                    vector_effect: "non-scaling-stroke",
                }
                for (index, (point, x, y)) in chart_points.iter().enumerate() {
                    {
                        let step_val = point.display_step;
                        let coord_step = point.coord_step;
                        let duration_ms = point.duration_ms;
                        let is_selected = selected_step()
                            .map(|selected| selected.rank == rank && selected.display_step == step_val)
                            .unwrap_or(false);
                        let visible = is_selected || index == latest_idx;
                        rsx! {
                            circle {
                                cx: "{x}", cy: "{y}", r: if visible { "4" } else { "7" },
                                fill: if visible { "#2563eb" } else { "transparent" },
                                stroke: if is_selected { "#1e3a8a" } else { "transparent" },
                                stroke_width: "2",
                                class: "cursor-pointer",
                                onclick: move |_| {
                                    select_training_step(
                                        rank,
                                        step_val,
                                        coord_step,
                                        duration_ms,
                                        selected_step,
                                    );
                                },
                                title { "step {step_val}: {duration_ms:.1} ms" }
                            }
                        }
                    }
                }
                text {
                    x: "{pad_left}", y: "{height - 6.0}",
                    text_anchor: "start", font_size: "9", fill: "#9ca3af", "step {first_step}"
                }
                text {
                    x: "{pad_left + plot_width}", y: "{height - 6.0}",
                    text_anchor: "end", font_size: "9", fill: "#9ca3af", "step {last_step}"
                }
            }
            div { class: "mt-1 flex justify-end gap-4 text-[9px] text-gray-400",
                span { class: "flex items-center gap-1",
                    span { class: "inline-block h-px w-4 bg-blue-600" }
                    "duration"
                }
                span { class: "flex items-center gap-1",
                    span { class: "inline-block w-4 border-t border-dashed border-gray-400" }
                    "average"
                }
            }
        }
    }
}

fn render_module_hotspots(
    modules: &Result<probing_proto::prelude::DataFrame, AppError>,
    phases: &Result<probing_proto::prelude::DataFrame, AppError>,
) -> Element {
    let has_modules = modules
        .as_ref()
        .ok()
        .map(|df| dataframe_rows(df) > 0)
        .unwrap_or(false);
    let has_phases = phases
        .as_ref()
        .ok()
        .map(|df| dataframe_rows(df) > 0)
        .unwrap_or(false);

    if !has_modules && !has_phases {
        return rsx! {
            CollapsibleCardWithIcon {
                title: "Module Hotspots".to_string(),
                icon: rsx! {
                    Icon { icon: &icondata::AiFireOutlined, class: "w-4 h-4 text-gray-500" }
                },
                children: rsx! {
                    EmptyState {
                        message: "No python.torch_trace data — SET probing.torch.profiling=on for module-level step breakdown.".to_string()
                    }
                },
            }
        };
    }

    if let Err(err) = modules {
        return rsx! {
            CollapsibleCardWithIcon {
                title: "Module Hotspots".to_string(),
                icon: rsx! {
                    Icon { icon: &icondata::AiFireOutlined, class: "w-4 h-4 text-gray-500" }
                },
                children: rsx! {
                    AppErrorDisplay { error: err.clone(), title: None }
                },
            }
        };
    }

    let row_count = modules.as_ref().ok().map(dataframe_rows).unwrap_or(0);

    rsx! {
        CollapsibleCardWithIcon {
            title: "Module Hotspots".to_string(),
            badge: if row_count > 0 { Some(format!("{row_count} modules")) } else { None },
            accent_border: Some("border-l-orange-400".to_string()),
            default_open: true,
            icon: rsx! {
                Icon { icon: &icondata::AiFireOutlined, class: "w-4 h-4 text-orange-600" }
            },
            children: rsx! {
                div { class: "space-y-4",
                    p { class: "text-xs text-gray-500",
                        "Top modules by post-hook time in the last 10 training steps · steps = distinct steps where this module was sampled · hooks = raw hook records (TorchProbe random mode samples a subset of modules each step)"
                    }
                    if has_modules {
                        if let Ok(df) = modules {
                            div { class: "overflow-x-auto border border-gray-200 rounded-lg max-h-72",
                                DataFrameView { df: df.clone() }
                            }
                        }
                    }
                    if has_phases {
                        div { class: "space-y-2",
                            p { class: "text-xs font-medium text-gray-700", "Forward vs optimizer (recent steps)" }
                            if let Ok(df) = phases {
                                div { class: "overflow-x-auto border border-gray-200 rounded-lg max-h-48",
                                    DataFrameView { df: df.clone() }
                                }
                            }
                        }
                    }
                    p { class: "text-xs text-gray-400",
                        "Select a slow step above, or use Investigate → Bottleneck skill for deeper analysis."
                    }
                }
            },
        }
    }
}

fn comm_table_collapsible(
    df: &probing_proto::prelude::DataFrame,
    summary: Option<&probing_proto::prelude::DataFrame>,
    scope_note: &str,
) -> Element {
    let rows = dataframe_rows(df);
    let df_for_click = df.clone();
    rsx! {
        CollapsibleCardWithIcon {
            title: "Collective Communications".to_string(),
            badge: Some(format!("{rows} rows")),
            accent_border: Some("border-l-violet-400".to_string()),
            icon: rsx! {
                Icon { icon: &icondata::AiClusterOutlined, class: "w-4 h-4 text-violet-600" }
            },
            children: rsx! {
                div { class: "space-y-3",
                    p { class: "text-xs text-gray-500", "{scope_note}" }
                    if let Some(summary_df) = summary {
                        div { class: "space-y-2",
                            p { class: "text-xs font-medium text-gray-700", "By collective op (aggregated)" }
                            div { class: "overflow-x-auto border border-gray-200 rounded-lg",
                                DataFrameView { df: summary_df.clone() }
                            }
                        }
                    }
                    p { class: "text-[10px] text-gray-500",
                        "Click a row to set investigation context (rank / op columns)."
                    }
                    div { class: "overflow-x-auto border border-gray-200 rounded-lg max-h-96",
                        DataFrameView {
                            df: df.clone(),
                            on_row_click: EventHandler::new(move |row: usize| {
                                apply_context_from_dataframe_row(&df_for_click, row);
                            }),
                        }
                    }
                    p { class: "text-xs text-gray-400",
                        "Lite mode: timing + context in python.comm_collective · full spans: SET probing.torch.collective.mode=full"
                    }
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn placement_node(host: &str, rank: i32, local_rank: i32, role: Option<&str>) -> Node {
        Node {
            host: host.to_string(),
            addr: format!("127.0.0.1:{}", 9000 + rank),
            rank: Some(rank),
            local_rank: Some(local_rank),
            world_size: Some(4),
            role: role.map(str::to_string),
            status: Some("running".to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn parallel_coordinates_use_stable_megatron_order() {
        let coordinates = parse_parallel_coordinates(Some("tp=1, dp=2,ignored=x,pp=0,sp=1"));

        assert_eq!(
            coordinates,
            vec![
                ("dp".to_string(), "2".to_string()),
                ("pp".to_string(), "0".to_string()),
                ("tp".to_string(), "1".to_string()),
                ("sp".to_string(), "1".to_string()),
            ]
        );
    }

    #[test]
    fn placement_matches_cpu_megatron_mock_topology() {
        let nodes = vec![
            placement_node("cpu-worker", 3, 3, Some("dp=0,pp=1,tp=1")),
            placement_node("cpu-worker", 1, 1, Some("dp=0,pp=0,tp=1")),
            placement_node("cpu-worker", 2, 2, Some("dp=0,pp=1,tp=0")),
            placement_node("cpu-worker", 0, 0, Some("dp=0,pp=0,tp=0")),
        ];

        let placement = build_placement(&nodes);

        assert_eq!(placement.observed_ranks, 4);
        assert_eq!(placement.expected_ranks, 4);
        assert!(placement.has_parallel_coordinates);
        assert_eq!(placement.hosts.len(), 1);
        assert_eq!(placement.hosts[0].name, "cpu-worker");
        assert_eq!(placement.hosts[0].processes[0].rank, Some(0));
        assert_eq!(placement.hosts[0].processes[3].rank, Some(3));
        assert_eq!(
            placement.parallel_sizes,
            vec![
                ("dp".to_string(), 1),
                ("pp".to_string(), 2),
                ("tp".to_string(), 2),
            ]
        );
        assert_eq!(
            placement.hosts[0].processes[3].coordinates,
            vec![
                ("dp".to_string(), "0".to_string()),
                ("pp".to_string(), "1".to_string()),
                ("tp".to_string(), "1".to_string()),
            ]
        );
    }

    #[test]
    fn placement_summarizes_64_rank_megatron_topology() {
        let nodes = (0..64)
            .map(|rank| {
                let tp = rank % 2;
                let pp = (rank / 2) % 4;
                let dp = rank / 8;
                let mut node = placement_node(
                    &format!("worker-{:02}", rank / 8),
                    rank,
                    rank % 8,
                    Some(&format!("dp={dp},pp={pp},sp={tp},tp={tp}")),
                );
                node.world_size = Some(64);
                node
            })
            .collect::<Vec<_>>();

        let placement = build_placement(&nodes);

        assert_eq!(placement.hosts.len(), 8);
        assert_eq!(placement_host_columns(placement.hosts.len()), 8);
        assert!(placement.hosts.iter().all(|host| host.processes.len() == 8));
        assert_eq!(placement.observed_ranks, 64);
        assert_eq!(placement.expected_ranks, 64);
        assert_eq!(
            placement.parallel_sizes,
            vec![
                ("dp".to_string(), 8),
                ("pp".to_string(), 4),
                ("tp".to_string(), 2),
                ("sp".to_string(), 2),
            ]
        );
        assert_eq!(placement.hosts[7].processes[7].rank, Some(63));
        assert_eq!(
            placement.hosts[7].processes[7].coordinates,
            vec![
                ("dp".to_string(), "7".to_string()),
                ("pp".to_string(), "3".to_string()),
                ("tp".to_string(), "1".to_string()),
                ("sp".to_string(), "1".to_string()),
            ]
        );

        let active = &placement.hosts[0].processes[0];
        assert_eq!(
            placement_group_membership(active, &placement.hosts[0].processes[0]),
            Some(PlacementGroup::Focus)
        );
        assert_eq!(
            placement_group_membership(active, &placement.hosts[0].processes[1]),
            Some(PlacementGroup::Tensor)
        );
        assert_eq!(
            placement_group_membership(active, &placement.hosts[0].processes[2]),
            Some(PlacementGroup::Pipeline)
        );
        assert_eq!(
            placement_group_membership(active, &placement.hosts[1].processes[0]),
            Some(PlacementGroup::Data)
        );
        assert_eq!(
            placement_group_membership(active, &placement.hosts[0].processes[3]),
            None
        );
        assert_eq!(
            placement_group_sizes(&placement, active),
            Some(PlacementGroupSizes {
                tensor: 2,
                data: 8,
                pipeline: 4,
            })
        );
    }

    #[test]
    fn placement_wraps_after_eight_host_columns() {
        assert_eq!(placement_host_columns(1), 1);
        assert_eq!(placement_host_columns(8), 8);
        assert_eq!(placement_host_columns(9), 8);
        assert_eq!(placement_host_columns(32), 8);
    }
}
