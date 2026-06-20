//! Training observability: local step/collective views + on-demand cluster scan.

use std::collections::{HashMap, HashSet};

use dioxus::prelude::*;

use crate::agent::load_skill;
use crate::api::{ApiClient, ClusterQueryResponse, StepDurationSample, StepMatrixResponse};
use crate::components::card::Card;
use crate::components::collapsible_card::CollapsibleCardWithIcon;
use crate::components::common::{AppErrorDisplay, AsyncBoundary, EmptyState, LoadingState};
use crate::components::dataframe_view::DataFrameView;
use crate::components::icon::Icon;
use crate::components::page::{PageContainer, PageTitle};
use crate::components::poll_status::{PollStatusBar, RefreshButton};
use crate::components::stat_card::StatCard;
use crate::components::workspace::{ChipButton, WidthSegment};
use crate::hooks::{use_app_resource, use_page_visible, use_poll_tick_gated};
use crate::state::agent::{AGENT_INPUT, AGENT_PANEL_OPEN};
use crate::state::investigation::{apply_context_from_dataframe_row, set_training_step_context};
use crate::state::ui_tasks::ui_agent_busy;
use crate::utils::error::AppError;

const POLL_MS: u32 = 5000;
const STEP_LIMIT: usize = 120;
const COMM_LIMIT: usize = 30;

const COMM_SQL: &str = "SELECT local_step, rank, op, group_size, duration_ms, bytes, role \
     FROM python.comm_collective ORDER BY timestamp DESC LIMIT ";

const COMM_SUMMARY_SQL: &str = "SELECT op, count(*) AS n, \
     round(avg(duration_ms), 2) AS avg_ms, round(max(duration_ms), 2) AS max_ms, \
     sum(bytes) AS total_bytes \
     FROM python.comm_collective GROUP BY op ORDER BY avg_ms DESC LIMIT 10";

const MODULE_HOTSPOTS_SQL: &str = "SELECT module, stage, count(DISTINCT step) AS steps, \
     count(*) AS hooks, round(avg(duration), 4) AS avg_sec, round(sum(duration), 4) AS total_sec \
     FROM python.torch_trace \
     WHERE step >= GREATEST(COALESCE((SELECT max(step) FROM python.torch_trace), 0) - 9, 1) \
       AND stage LIKE 'post %' AND duration > 0 \
       AND module IS NOT NULL AND module != '' AND module != 'None' \
     GROUP BY module, stage ORDER BY total_sec DESC LIMIT 12";

const STEP_PHASE_SQL: &str = "SELECT step, \
     round(sum(CASE WHEN stage = 'post forward' THEN duration ELSE 0 END), 4) AS forward_sec, \
     round(sum(CASE WHEN stage = 'post step' THEN duration ELSE 0 END), 4) AS optim_sec \
     FROM python.torch_trace \
     WHERE step >= GREATEST(COALESCE((SELECT max(step) FROM python.torch_trace), 0) - 15, 1) \
       AND stage LIKE 'post %' AND duration > 0 \
       AND module IS NOT NULL AND module != '' AND module != 'None' \
     GROUP BY step ORDER BY step";

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
         WHERE step = {coord_step} AND stage LIKE 'post %' AND duration > 0 \
           AND module IS NOT NULL AND module != '' AND module != 'None' \
         ORDER BY duration DESC LIMIT 12"
    )
}

fn step_span_sql(display_step: i64) -> String {
    format!(
        "SELECT s.name, s.kind, round((e.time - s.time) / 1000000.0, 2) AS duration_ms \
         FROM python.trace_event s \
         JOIN python.trace_event e ON s.span_id = e.span_id AND e.record_type = 'span_end' \
         WHERE s.record_type = 'span_start' AND s.kind != 'train.step' \
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
pub fn Training() -> Element {
    let visible = use_page_visible();
    let poll = use_poll_tick_gated(POLL_MS, Some(visible));
    let mut manual_refresh = use_signal(|| 0u32);
    let local_tick = poll().wrapping_add(manual_refresh());

    let mut scope = use_signal(|| DataScope::Local);
    let nodes = use_app_resource(|| async move { ApiClient::new().get_nodes().await });
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

    let peer_count = nodes()
        .and_then(|r| r.ok())
        .map(|nodes| nodes.len().saturating_sub(1))
        .unwrap_or(0);

    let current_scope = scope();
    let scan_pending = cluster_scan.pending();
    let selected_step = use_signal(|| None::<SelectedStep>);

    rsx! {
        PageContainer {
            PageTitle {
                title: "Training".to_string(),
                subtitle: Some("Step timing, per-step breakdown, module hooks, and collective latency.".to_string()),
                icon: Some(&icondata::AiRadarChartOutlined),
                header_right: Some(rsx! {
                    if current_scope == DataScope::Local {
                        PollStatusBar {
                            interval_secs: POLL_MS / 1000,
                            poll_tick: local_tick,
                        }
                    }
                    RefreshButton {
                        onclick: move |_| {
                            if current_scope == DataScope::Cluster {
                                cluster_scan.call();
                            } else {
                                manual_refresh.set(manual_refresh() + 1);
                            }
                        },
                    }
                }),
            }

            TrainingScopeBar {
                scope: current_scope,
                peer_count,
                scan_pending,
                on_local: move |_| scope.set(DataScope::Local),
                on_cluster_scan: move |_| {
                    scope.set(DataScope::Cluster);
                    cluster_scan.call();
                },
            }

            if current_scope == DataScope::Cluster {
                if let Some(Ok(output)) = cluster_scan.value() {
                    {cluster_nodes_failed_banner(&output().nodes_failed)}
                }
            }

            div { class: "space-y-4 min-w-0",
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
                        title: "Step timings",
                        LoadingState { message: Some("Scanning cluster…".to_string()) }
                    }
                } else if let Some(Err(err)) = cluster_scan.value() {
                    Card {
                        title: "Step timings",
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
                        title: "Step timings",
                        EmptyState { message: "Run a cluster scan to compare ranks.".to_string() }
                    }
                }
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
    let min = durations.iter().copied().fold(f64::INFINITY, f64::min);
    let max = durations.iter().copied().fold(0.0f64, f64::max);
    let p95 = percentile(&durations, 0.95);
    let (latest_step, latest_ms) = series.last().copied().unwrap_or((-1, 0.0));

    let (trend_value, trend_hint) = if series.len() >= 6 {
        let mid = series.len() / 2;
        let first_half = series[..mid].iter().map(|(_, d)| d).sum::<f64>() / mid.max(1) as f64;
        let second_half =
            series[mid..].iter().map(|(_, d)| d).sum::<f64>() / (series.len() - mid).max(1) as f64;
        let pct = (second_half - first_half) / first_half.max(1.0) * 100.0;
        if pct.abs() < 3.0 {
            (
                "Stable".to_string(),
                Some("Second half vs first half of window".to_string()),
            )
        } else if pct > 0.0 {
            (
                format!("+{pct:.0}% slower"),
                Some("Second half vs first half of window".to_string()),
            )
        } else {
            (
                format!("{pct:.0}% faster"),
                Some("Second half vs first half of window".to_string()),
            )
        }
    } else {
        ("—".to_string(), None)
    };

    vec![
        (
            "Latest step".to_string(),
            latest_step.to_string(),
            Some(format_step_ms(latest_ms)),
        ),
        ("Avg step".to_string(), format_step_ms(avg), None),
        (
            "Min / Max".to_string(),
            format!("{} / {}", format_step_ms(min), format_step_ms(max)),
            None,
        ),
        ("P95".to_string(), format_step_ms(p95), None),
        ("Trend".to_string(), trend_value, trend_hint),
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
                title: "Step timings",
                EmptyState {
                    message: "No train.step spans yet. Wrap training loops with probing.span(..., kind='train.step') or enable TorchProbe.".to_string()
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
            let title = if single_rank {
                "Step timings"
            } else {
                "Step straggler heatmap"
            };
            let legend = if single_rank {
                "Bar height = train.step duration · red = >1.2× window avg · click to inspect"
            } else {
                "Darker = slower · red ring = outlier (>1.2× step median) · click to inspect"
            };
            let rank = primary_rank(&resp.samples);
            rsx! {
                Card {
                    title: title,
                    content_class: Some("p-4"),
                    div { class: "space-y-4",
                        div { class: "grid grid-cols-2 sm:grid-cols-5 gap-3",
                            for (label, value, hint) in stats {
                                StatCard { label, value, hint }
                            }
                        }
                        div { class: "flex flex-wrap items-center gap-4 text-sm text-gray-600",
                            span { "{scope_note}" }
                            if single_rank {
                                span { "·" }
                                span { class: "text-xs font-mono text-violet-700 bg-violet-50 px-2 py-0.5 rounded",
                                    "rank {rank} · single-process view"
                                }
                            }
                            span { "·" }
                            span { class: "text-xs text-gray-500", "{legend}" }
                        }
                        if single_rank {
                            StepDurationTimeline {
                                rank,
                                points: build_step_points(&resp.samples),
                                selected_step,
                            }
                        } else {
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
                title: "Step timings",
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
                        message: "No collective samples on this node. Auto-enabled for torchrun (WORLD_SIZE>1).".to_string()
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
    df.cols.first().map(|c| c.len()).unwrap_or(0)
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

    let max_ms = points
        .iter()
        .map(|p| p.duration_ms)
        .fold(0.0f64, f64::max)
        .max(1.0);
    let avg_ms = points.iter().map(|p| p.duration_ms).sum::<f64>() / points.len() as f64;
    let latest_idx = points.len().saturating_sub(1);
    let detail_rows: Vec<_> = points.iter().rev().take(12).cloned().collect();

    rsx! {
        div { class: "space-y-4",
            div { class: "overflow-x-auto pb-1",
                div { class: "flex items-end gap-1 min-w-max h-36 px-1",
                    for (i, point) in points.iter().enumerate() {
                        {
                            let is_latest = i == latest_idx;
                            let dur = point.duration_ms;
                            let pct = (dur / max_ms).clamp(0.08, 1.0);
                            let slow = dur > avg_ms * 1.2;
                            let bar_color = if slow {
                                "bg-red-500/90 hover:bg-red-600"
                            } else {
                                "bg-violet-500/85 hover:bg-violet-600"
                            };
                            let is_selected = selected_step()
                                .map(|s| s.rank == rank && s.display_step == point.display_step)
                                .unwrap_or(false);
                            let ring = if is_selected {
                                "ring-2 ring-blue-500 ring-offset-1"
                            } else if is_latest {
                                "ring-2 ring-blue-400 ring-offset-1"
                            } else if slow {
                                "ring-1 ring-red-300"
                            } else {
                                ""
                            };
                            let step_val = point.display_step;
                            let coord_step = point.coord_step;
                            let title = format!("step {step_val}: {dur:.1} ms — click to inspect");
                            rsx! {
                                button {
                                    r#type: "button",
                                    class: "flex flex-col items-center justify-end gap-1 min-w-[36px] h-full group",
                                    title: "{title}",
                                    onclick: move |_| {
                                        select_training_step(
                                            rank,
                                            step_val,
                                            coord_step,
                                            dur,
                                            selected_step,
                                        );
                                    },
                                    span {
                                        class: "text-[9px] font-mono text-gray-500 opacity-0 group-hover:opacity-100 transition-opacity",
                                        "{dur:.0}"
                                    }
                                    div {
                                        class: "w-7 rounded-t-sm {bar_color} {ring} transition-colors",
                                        style: "height: {pct * 100.0}%",
                                    }
                                    span {
                                        class: if is_selected || is_latest {
                                            "text-[10px] font-mono font-semibold text-blue-700"
                                        } else {
                                            "text-[10px] font-mono text-gray-500"
                                        },
                                        "{step_val}"
                                    }
                                }
                            }
                        }
                    }
                }
            }

            div { class: "border border-gray-200 rounded-lg overflow-hidden",
                table { class: "w-full text-xs",
                    thead {
                        tr { class: "bg-gray-50 text-gray-500",
                            th { class: "px-3 py-2 text-left font-medium", "Step" }
                            th { class: "px-3 py-2 text-right font-medium", "Duration" }
                            th { class: "px-3 py-2 text-right font-medium", "vs avg" }
                        }
                    }
                    tbody {
                        for point in detail_rows {
                            {
                                let dur = point.duration_ms;
                                let delta_pct = (dur - avg_ms) / avg_ms.max(1.0) * 100.0;
                                let delta_label = if delta_pct.abs() < 1.0 {
                                    "±0%".to_string()
                                } else if delta_pct > 0.0 {
                                    format!("+{delta_pct:.0}%")
                                } else {
                                    format!("{delta_pct:.0}%")
                                };
                                let delta_class = if delta_pct > 10.0 {
                                    "text-red-600 font-medium"
                                } else if delta_pct < -10.0 {
                                    "text-emerald-600 font-medium"
                                } else {
                                    "text-gray-500"
                                };
                                let is_selected = selected_step()
                                    .map(|s| s.rank == rank && s.display_step == point.display_step)
                                    .unwrap_or(false);
                                let row_class = if is_selected {
                                    "border-t border-gray-100 bg-blue-50 cursor-pointer"
                                } else {
                                    "border-t border-gray-100 hover:bg-gray-50 cursor-pointer"
                                };
                                let step_val = point.display_step;
                                let coord_step = point.coord_step;
                                rsx! {
                                    tr {
                                        class: "{row_class}",
                                        onclick: move |_| {
                                            select_training_step(
                                                rank,
                                                step_val,
                                                coord_step,
                                                dur,
                                                selected_step,
                                            );
                                        },
                                        td { class: "px-3 py-1.5 font-mono text-gray-800", "{step_val}" }
                                        td { class: "px-3 py-1.5 text-right font-mono text-gray-800", "{dur:.1} ms" }
                                        td { class: "px-3 py-1.5 text-right font-mono {delta_class}", "{delta_label}" }
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
