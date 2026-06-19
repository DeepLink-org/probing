//! Training observability: local step/collective views + on-demand cluster scan.

use std::collections::{HashMap, HashSet};

use dioxus::prelude::*;

use crate::api::{ApiClient, ClusterQueryResponse, StepDurationSample, StepMatrixResponse};
use crate::components::card::Card;
use crate::components::common::{EmptyState, ErrorState, LoadingState};
use crate::components::page::{PageContainer, PageTitle};
use crate::hooks::{use_api, use_api_simple, use_poll_tick};

const POLL_MS: u32 = 5000;
const STEP_LIMIT: usize = 120;
const COMM_LIMIT: usize = 30;

const COMM_SQL: &str = "SELECT local_step, rank, op, group_size, duration_ms, bytes, tp_rank, pp_rank, dp_rank \
     FROM python.comm_collective ORDER BY timestamp DESC LIMIT ";

#[derive(Clone, Copy, PartialEq, Eq)]
enum DataScope {
    Local,
    Cluster,
}

#[derive(Clone, Debug, PartialEq)]
struct HeatCell {
    duration_ms: f64,
    outlier: bool,
}

#[component]
pub fn Training() -> Element {
    let poll = use_poll_tick(POLL_MS);
    let mut scope = use_signal(|| DataScope::Local);
    let mut cluster_scan_gen = use_signal(|| 0u32);
    let nodes_state = use_api(|| {
        let client = ApiClient::new();
        async move { client.get_nodes().await }
    });

    let local_matrix = use_api(move || {
        let _ = poll();
        let client = ApiClient::new();
        async move { client.fetch_step_matrix(STEP_LIMIT, false).await }
    });

    let local_comm = use_api(move || {
        let _ = poll();
        let client = ApiClient::new();
        async move {
            client
                .execute_query(&format!("{COMM_SQL}{COMM_LIMIT}"))
                .await
        }
    });

    let cluster_matrix = use_api_simple::<StepMatrixResponse>();
    let cluster_comm = use_api_simple::<ClusterQueryResponse>();
    let cluster_nodes_failed = use_signal(Vec::<String>::new);

    let peer_count = nodes_state
        .data
        .read()
        .as_ref()
        .and_then(|r| r.as_ref().ok())
        .map(|nodes| nodes.len().saturating_sub(1))
        .unwrap_or(0);

    let current_scope = scope();

    rsx! {
        PageContainer {
            PageTitle {
                title: "Training".to_string(),
                subtitle: Some("Local train.step and collective traces refresh automatically; cluster-wide views run on demand.".to_string()),
                icon: Some(&icondata::AiRadarChartOutlined),
            }
            div { class: "flex flex-wrap items-center gap-3 mb-4",
                {scope_badge(current_scope)}
                button {
                    class: "px-3 py-1.5 text-sm rounded-md border border-gray-300 bg-white hover:bg-gray-50 disabled:opacity-50",
                    disabled: peer_count == 0,
                    title: if peer_count == 0 { "No peer nodes in cluster view" } else { "Query all cluster nodes once (may take a few seconds)" },
                    onclick: move |_| {
                        scope.set(DataScope::Cluster);
                        cluster_scan_gen.set(cluster_scan_gen() + 1);
                        let gen = cluster_scan_gen();
                        let mut matrix_state = cluster_matrix.clone();
                        let mut comm_state = cluster_comm.clone();
                        let mut failed_signal = cluster_nodes_failed;
                        spawn(async move {
                            let _ = gen;
                            *matrix_state.loading.write() = true;
                            *comm_state.loading.write() = true;
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
                            failed_signal.set(merged);

                            *matrix_state.data.write() = Some(matrix_res);
                            *comm_state.data.write() = Some(comm_res);
                            *matrix_state.loading.write() = false;
                            *comm_state.loading.write() = false;
                        });
                    },
                    if peer_count == 0 {
                        "Scan cluster (no peers)"
                    } else {
                        "Scan cluster ({peer_count} peers)"
                    }
                }
                if current_scope == DataScope::Cluster {
                    button {
                        class: "px-3 py-1.5 text-sm rounded-md border border-violet-200 text-violet-800 bg-violet-50 hover:bg-violet-100",
                        onclick: move |_| scope.set(DataScope::Local),
                        "Back to local only"
                    }
                }
                p { class: "text-xs text-gray-500",
                    "Agents write locally only. Fan-out runs when you click Scan cluster or use probing cluster query."
                }
            }
            if current_scope == DataScope::Cluster {
                {cluster_nodes_failed_banner(&cluster_nodes_failed)}
            }
            {step_matrix_panel(current_scope, &local_matrix, &cluster_matrix)}
            {comm_panel(current_scope, &local_comm, &cluster_comm)}
        }
    }
}

fn scope_badge(scope: DataScope) -> Element {
    let (label, class) = match scope {
        DataScope::Local => ("Scope: this node", "bg-gray-100 text-gray-700"),
        DataScope::Cluster => ("Scope: cluster scan", "bg-violet-100 text-violet-800"),
    };
    rsx! {
        span { class: "text-xs font-medium px-2 py-1 rounded {class}", "{label}" }
    }
}

fn cluster_nodes_failed_banner(failed: &Signal<Vec<String>>) -> Element {
    let nodes = failed.read().clone();
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

fn step_matrix_panel(
    scope: DataScope,
    local: &crate::hooks::ApiState<StepMatrixResponse>,
    cluster: &crate::hooks::ApiState<StepMatrixResponse>,
) -> Element {
    let state = if scope == DataScope::Cluster { cluster } else { local };

    if state.is_loading() {
        return rsx! {
            Card {
                title: "Step Straggler Heatmap",
                LoadingState { message: Some("Loading train.step matrix…".to_string()) }
            }
        };
    }

    match state.data.read().as_ref() {
        Some(Ok(resp)) if !resp.samples.is_empty() => {
            let (ranks, steps, cells, max_ms) = build_heatmap(&resp.samples);
            let scope_note = if resp.cluster {
                let mut note = format!("cluster scan · {} nodes queried", resp.nodes_queried);
                if !resp.nodes_failed.is_empty() {
                    note.push_str(&format!(
                        " · {} failed",
                        resp.nodes_failed.len()
                    ));
                }
                note
            } else {
                "local node · auto-refresh".to_string()
            };
            rsx! {
                Card {
                    title: "Step Straggler Heatmap",
                    content_class: Some("p-4"),
                    div { class: "space-y-4",
                        div { class: "flex flex-wrap items-center gap-4 text-sm text-gray-600",
                            span { "{scope_note}" }
                            span { "·" }
                            span { "{resp.rank_count} ranks" }
                            span { "·" }
                            span { "{resp.step_count} steps sampled" }
                            span { "·" }
                            span { class: "text-xs text-gray-500",
                                "Darker = slower · red ring = outlier (>1.2× step median)"
                            }
                        }
                        StepHeatmap {
                            ranks: ranks.clone(),
                            steps: steps.clone(),
                            cells: cells.clone(),
                            max_ms,
                        }
                    }
                }
            }
        }
        Some(Ok(_)) => rsx! {
            Card {
                title: "Step Straggler Heatmap",
                EmptyState {
                    message: "No train.step spans yet. Wrap training loops with probing.span(..., kind='train.step') or enable TorchProbe.".to_string()
                }
            }
        },
        Some(Err(e)) => rsx! {
            Card {
                title: "Step Straggler Heatmap",
                ErrorState { error: e.display_message(), title: None }
            }
        },
        _ => rsx! { div {} },
    }
}

fn build_heatmap(samples: &[StepDurationSample]) -> (Vec<i32>, Vec<i64>, HashMap<(i32, i64), HeatCell>, f64) {
    let mut rank_set = HashSet::new();
    let mut step_set = HashSet::new();
    let mut raw: HashMap<(i32, i64), f64> = HashMap::new();

    for s in samples {
        if s.rank < 0 || s.local_step < 0 {
            continue;
        }
        rank_set.insert(s.rank);
        step_set.insert(s.local_step);
        raw.entry((s.rank, s.local_step))
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
        cells.insert((rank, step), HeatCell { duration_ms: dur, outlier });
    }

    (ranks, steps, cells, max_ms)
}

#[component]
fn StepHeatmap(
    ranks: Vec<i32>,
    steps: Vec<i64>,
    cells: HashMap<(i32, i64), HeatCell>,
    max_ms: f64,
) -> Element {
    let featured = ranks.len() <= 1;
    let cell_min = if featured { "min-w-[48px]" } else { "min-w-[28px]" };
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
                                    format!("rank {rank} step {step}: {:.1} ms", c.duration_ms),
                                    ring.to_string(),
                                )
                            } else {
                                (
                                    "background-color: rgb(243 244 246);".to_string(),
                                    format!("rank {rank} step {step}: no data"),
                                    String::new(),
                                )
                            };
                            rsx! {
                                div {
                                    class: "rounded-sm {cell_min} {cell_h} {ring}",
                                    style: "{bg}",
                                    title: "{title}",
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn comm_panel(
    scope: DataScope,
    local: &crate::hooks::ApiState<probing_proto::prelude::DataFrame>,
    cluster: &crate::hooks::ApiState<ClusterQueryResponse>,
) -> Element {
    if scope == DataScope::Cluster {
        comm_panel_cluster(cluster)
    } else {
        comm_panel_local(local)
    }
}

fn comm_panel_local(state: &crate::hooks::ApiState<probing_proto::prelude::DataFrame>) -> Element {
    if state.is_loading() {
        return rsx! {
            Card {
                title: "Collective Communications",
                content_class: Some("p-4"),
                LoadingState { message: Some("Loading comm.collective (local)…".to_string()) }
            }
        };
    }

    match state.data.read().as_ref() {
        Some(Ok(df)) if !df.cols.is_empty() => comm_table(df, "local node · auto-refresh"),
        Some(Ok(_)) => rsx! {
            Card {
                title: "Collective Communications",
                content_class: Some("p-4"),
                EmptyState {
                    message: "No collective samples on this node. Auto-enabled for torchrun (WORLD_SIZE>1).".to_string()
                }
            }
        },
        Some(Err(e)) => rsx! {
            Card {
                title: "Collective Communications",
                content_class: Some("p-4"),
                ErrorState { error: e.display_message(), title: None }
            }
        },
        _ => rsx! { div {} },
    }
}

fn comm_panel_cluster(state: &crate::hooks::ApiState<ClusterQueryResponse>) -> Element {
    if state.is_loading() {
        return rsx! {
            Card {
                title: "Collective Communications",
                content_class: Some("p-4"),
                LoadingState { message: Some("Scanning cluster for comm.collective…".to_string()) }
            }
        };
    }

    match state.data.read().as_ref() {
        Some(Ok(resp)) if !resp.dataframe.cols.is_empty() => {
            let mut note = format!(
                "cluster scan · {} nodes queried",
                resp.meta.nodes_queried
            );
            if !resp.meta.nodes_failed.is_empty() {
                note.push_str(&format!(
                    " · {} failed",
                    resp.meta.nodes_failed.len()
                ));
            }
            comm_table(&resp.dataframe, &note)
        }
        Some(Ok(_)) => rsx! {
            Card {
                title: "Collective Communications",
                content_class: Some("p-4"),
                EmptyState { message: "No collective rows returned from cluster scan.".to_string() }
            }
        },
        Some(Err(e)) => rsx! {
            Card {
                title: "Collective Communications",
                content_class: Some("p-4"),
                ErrorState { error: e.display_message(), title: None }
            }
        },
        _ => rsx! {
            Card {
                title: "Collective Communications",
                content_class: Some("p-4"),
                EmptyState { message: "Click Scan cluster to load cross-node collective rows.".to_string() }
            }
        },
    }
}

fn comm_table(df: &probing_proto::prelude::DataFrame, scope_note: &str) -> Element {
    let rows = df.cols.first().map(|c| c.len()).unwrap_or(0);
    rsx! {
        Card {
            title: "Collective Communications",
            content_class: Some("p-4"),
            p { class: "text-xs text-gray-500 mb-3", "{scope_note}" }
            div { class: "overflow-x-auto border border-gray-200 rounded-lg",
                table { class: "min-w-full text-sm",
                    thead { class: "bg-gray-50 text-left text-xs uppercase text-gray-500",
                        tr {
                            for name in df.names.iter() {
                                th { class: "px-3 py-2", "{name}" }
                            }
                        }
                    }
                    tbody {
                        for r in 0..rows {
                            tr { class: "border-t border-gray-100",
                                for (col_idx, _name) in df.names.iter().enumerate() {
                                    td { class: "px-3 py-2 font-mono text-xs",
                                        {cell_text(df, col_idx, r)}
                                    }
                                }
                            }
                        }
                    }
                }
            }
            p { class: "text-xs text-gray-400 mt-3",
                "Lite mode: timing + context in python.comm_collective · full spans: SET probing.torch.collective.mode=full"
            }
        }
    }
}

fn cell_text(df: &probing_proto::prelude::DataFrame, col: usize, row: usize) -> String {
    use probing_proto::prelude::Ele;
    match df.cols.get(col).map(|c| c.get(row)) {
        Some(Ele::Text(s)) => s.clone(),
        Some(Ele::I64(v)) => v.to_string(),
        Some(Ele::I32(v)) => v.to_string(),
        Some(Ele::F64(v)) => format!("{v:.2}"),
        Some(Ele::F32(v)) => format!("{v:.2}"),
        _ => "—".to_string(),
    }
}
