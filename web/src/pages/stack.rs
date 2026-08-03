use dioxus::prelude::*;
use dioxus_router::Link;
use probing_proto::prelude::CallFrame;

use crate::api::ApiClient;
use crate::app::Route;
use crate::components::callstack_view::CallStackView;
use crate::components::common::{AsyncBoundary, EmptyState, ErrorState};
use crate::components::flamegraph::{FlamegraphPayload, FlamegraphView};
use crate::components::page::{PageContainer, PageTitle};
use crate::components::profiling::{ProfilingContentPanel, ProfilingErrorPanel};
use crate::hooks::use_app_resource;
use crate::state::stack::{
    stack_tid_label, StackSnapshot, STACK_DIST_CLUSTER, STACK_DIST_RELOAD, STACK_MODE,
    STACK_REFRESH, STACK_SNAPSHOT,
};
use crate::utils::callframe::{
    classify_frame, count_by_kind, frame_location, frame_matches_query, frame_title, matches_mode,
    stack_evidence,
};
use crate::utils::error::AppError;

#[component]
pub fn Stack(tid: Option<String>) -> Element {
    let tid_for_api = tid.clone();
    let tid_label = stack_tid_label(tid.as_deref());
    let refresh_tick = STACK_REFRESH();

    rsx! {
        PageContainer {
            PageTitle {
                title: "Stacks".to_string(),
                subtitle: Some(format!(
                    "One live root-to-current call path · thread {tid_label}"
                )),
                icon: Some(&icondata::AiApartmentOutlined),
            }

            AsyncBoundary {
                message: Some("Loading call stack…".to_string()),
                StackLoaded {
                    tid: tid_for_api,
                    tid_label: tid_label,
                    refresh_tick: refresh_tick,
                }
            }
        }
    }
}

#[component]
fn StackLoaded(tid: Option<String>, tid_label: String, refresh_tick: u32) -> Element {
    let mode = STACK_MODE();
    let filter_mode = mode.clone();
    let mut query = use_signal(String::new);
    let stack = use_app_resource(move || {
        let _ = refresh_tick;
        let tid_arg = tid.clone();
        async move {
            ApiClient::new()
                .get_callstack_with_mode(tid_arg, "mixed")
                .await
        }
    });

    let stack_peek = stack.read().clone();
    let tid_for_effect = tid_label.clone();

    use_effect(use_reactive!(|(
        mode,
        refresh_tick,
        stack_peek,
        tid_for_effect,
    )| {
        let _ = refresh_tick;
        let Some(result) = stack_peek.as_ref() else {
            return;
        };
        *STACK_SNAPSHOT.write() = stack_snapshot_for(&tid_for_effect, result, &mode);
    }));

    match stack.suspend()?().as_ref() {
        Err(err) => rsx! {
            ErrorState {
                title: Some("Failed to load stack".to_string()),
                error: err.display_message(),
            }
        },
        Ok(callframes) if callframes.is_empty() => rsx! {
            EmptyState {
                message: format!(
                    "No stack frames for thread {tid_label}. The thread may be idle or not yet sampled."
                )
            }
        },
        Ok(callframes) => {
            let current_mode = filter_mode.clone();
            let query_value = query.read().trim().to_string();
            let filtered: Vec<_> = callframes
                .iter()
                .enumerate()
                .filter(|(_, frame)| matches_mode(frame, current_mode.as_str()))
                .filter(|(_, frame)| frame_matches_query(frame, &query_value))
                .map(|(index, frame)| (index, frame.clone()))
                .collect();
            let shown = filtered.len();
            let total = callframes.len();
            let current_is_visible = filtered.iter().any(|(index, _)| *index + 1 == total);

            rsx! {
                div { class: "space-y-4",
                    StackSnapshotCard {
                        frames: callframes.clone(),
                        tid_label: tid_label.clone(),
                        mode: current_mode.clone(),
                        query: query_value.clone(),
                    }
                    section { class: "overflow-hidden rounded-xl border border-gray-200 bg-white shadow-sm",
                        div { class: "flex flex-wrap items-center justify-between gap-3 border-b border-gray-100 px-4 py-2.5",
                            div {
                                h2 { class: "text-sm font-semibold text-gray-950", "Call path" }
                                p { class: "mt-0.5 text-xs text-gray-500",
                                    "{shown} of {total} frames · {mode_label(&current_mode)}"
                                }
                            }
                            input {
                                r#type: "search",
                                class: "w-64 max-w-full rounded-lg border border-gray-300 px-3 py-1.5 text-xs text-gray-800 focus:border-blue-500 focus:outline-none focus:ring-2 focus:ring-blue-500/20",
                                placeholder: "Find function or source file",
                                value: "{query_value}",
                                aria_label: "Find stack frame",
                                oninput: move |event| query.set(event.value()),
                            }
                        }
                        if filtered.is_empty() {
                            div { class: "p-4",
                                EmptyState {
                                    message: if query_value.is_empty() {
                                        format!("No {} frames in this capture", mode_label(&current_mode))
                                    } else {
                                        format!("No frames match \"{query_value}\"")
                                    }
                                }
                            }
                        } else {
                            div { class: "p-3",
                                for (visible_index, (frame_index, frame)) in filtered.iter().enumerate() {
                                    div {
                                        id: "stack-frame-{frame_index}",
                                        CallStackView {
                                            key: "{refresh_tick}-{frame_index}",
                                            callstack: frame.clone(),
                                            index: *frame_index,
                                            is_last: visible_index + 1 == shown,
                                            default_open: *frame_index + 1 == total || (!current_is_visible && visible_index == 0),
                                            position: frame_position(*frame_index, total),
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
fn StackSnapshotCard(
    frames: Vec<CallFrame>,
    tid_label: String,
    mode: String,
    query: String,
) -> Element {
    let evidence = stack_evidence(&frames);
    let source_coverage = format!("{} / {}", evidence.source_frames, evidence.total);
    let current = frames.last().cloned();

    rsx! {
        section { class: "overflow-hidden rounded-xl border border-gray-200 bg-white shadow-sm",
            div { class: "border-b border-gray-100 px-4 py-2.5",
                h2 { class: "text-sm font-semibold text-gray-950", "Captured stack" }
                p { class: "mt-0.5 text-xs text-gray-500", "Thread {tid_label} · one on-demand sample" }
            }
            div { class: "p-4 space-y-4",
                div { class: "grid grid-cols-4 divide-x divide-gray-200",
                    StackMetric { label: "Frames", value: evidence.total.to_string() }
                    StackMetric { label: "Runtime boundaries", value: evidence.runtime_boundaries.to_string() }
                    StackMetric { label: "Source locations", value: source_coverage }
                    StackMetric { label: "Python locals", value: evidence.local_values.to_string() }
                }
                if let Some(current) = current {
                    CurrentFrameEvidence { frame: current, index: evidence.total.saturating_sub(1) }
                }
                StackOverview { frames, mode, query }
            }
        }
    }
}

#[component]
fn StackMetric(label: &'static str, value: String) -> Element {
    rsx! {
        div { class: "min-w-0 px-3 first:pl-0 last:pr-0",
            div { class: "text-[10px] font-medium uppercase tracking-wide text-gray-500", "{label}" }
            div { class: "mt-1 text-lg font-semibold tabular-nums text-gray-950", "{value}" }
        }
    }
}

#[component]
fn CurrentFrameEvidence(frame: CallFrame, index: usize) -> Element {
    let title = frame_title(&frame);
    let kind = classify_frame(&frame);
    let (kind_label, kind_class) = kind.status_badge();
    let location = frame_location(&frame)
        .filter(|(file, _)| !file.is_empty())
        .map(|(file, line)| format!("{file}:{line}"));

    rsx! {
        div { class: "flex min-w-0 items-center gap-3 rounded-lg border border-gray-200 bg-gray-50 px-3 py-2",
            span { class: "shrink-0 text-[10px] font-medium uppercase tracking-wide text-gray-500", "Current frame" }
            span { class: "shrink-0 font-mono text-[10px] tabular-nums text-gray-400", "#{index}" }
            span { class: "min-w-0 flex-1 truncate font-mono text-xs font-medium text-gray-900", title: "{title}", "{title}" }
            if let Some(location) = location {
                span { class: "hidden max-w-[35%] truncate font-mono text-[10px] text-gray-500 xl:block", title: "{location}", "{location}" }
            }
            span { class: "shrink-0 rounded border px-1.5 py-0.5 text-[9px] font-semibold {kind_class}", "{kind_label}" }
        }
    }
}

#[component]
fn StackOverview(frames: Vec<CallFrame>, mode: String, query: String) -> Element {
    let evidence = stack_evidence(&frames);
    rsx! {
        div { class: "space-y-2 border-t border-gray-100 pt-3",
            div { class: "flex flex-wrap items-center justify-between gap-2 text-[10px] text-gray-500",
                span { "Root → current · one cell per frame" }
                div { class: "flex flex-wrap items-center gap-3",
                    StackLegend { class: "bg-emerald-500", label: format!("Python {}", evidence.python) }
                    StackLegend { class: "bg-orange-500", label: format!("Rust {}", evidence.rust) }
                    StackLegend { class: "bg-blue-500", label: format!("Native {}", evidence.native) }
                }
            }
            div { class: "grid grid-flow-col auto-cols-fr gap-1 overflow-x-auto pb-1",
                for (index, frame) in frames.iter().enumerate() {
                    {
                        let active = matches_mode(frame, &mode) && frame_matches_query(frame, &query);
                        let title = frame_overview_title(index, frame);
                        let classes = if active {
                            classify_frame(frame).overview_cell()
                        } else {
                            "bg-gray-200"
                        };
                        rsx! {
                            if active {
                                a {
                                    href: "#stack-frame-{index}",
                                    class: "h-4 min-w-2 rounded-sm {classes} focus:outline-none focus:ring-2 focus:ring-blue-500 focus:ring-offset-1",
                                    title: "{title}",
                                    aria_label: "Open frame {index}: {frame_title(frame)}",
                                }
                            } else {
                                span { class: "h-4 min-w-2 rounded-sm {classes}", title: "{title}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn StackLegend(class: &'static str, label: String) -> Element {
    rsx! {
        span { class: "inline-flex items-center gap-1",
            span { class: "h-2 w-2 rounded-sm {class}" }
            "{label}"
        }
    }
}

fn frame_overview_title(index: usize, frame: &CallFrame) -> String {
    let location = frame_location(frame)
        .filter(|(file, _)| !file.is_empty())
        .map(|(file, line)| format!(" · {file}:{line}"))
        .unwrap_or_default();
    format!("#{index} · {}{location}", frame_title(frame))
}

fn frame_position(index: usize, total: usize) -> Option<String> {
    match (index == 0, index + 1 == total) {
        (true, true) => Some("root · current".to_string()),
        (true, false) => Some("root".to_string()),
        (false, true) => Some("current".to_string()),
        (false, false) => None,
    }
}

fn stack_snapshot_for(
    tid_label: &str,
    result: &Result<Vec<CallFrame>, AppError>,
    mode: &str,
) -> StackSnapshot {
    match result {
        Err(_) => StackSnapshot::default(),
        Ok(frames) if frames.is_empty() => StackSnapshot {
            tid_label: tid_label.to_string(),
            loaded: true,
            ..StackSnapshot::default()
        },
        Ok(frames) => {
            let (py_count, rust_count, cpp_count) = count_by_kind(frames);
            let shown = frames.iter().filter(|cf| matches_mode(cf, mode)).count();
            StackSnapshot {
                tid_label: tid_label.to_string(),
                total: frames.len(),
                py: py_count,
                rust: rust_count,
                cpp: cpp_count,
                shown,
                loaded: true,
            }
        }
    }
}

fn mode_label(mode: &str) -> &'static str {
    match mode {
        "py" => "Python",
        "rust" => "Rust",
        "cpp" => "Native",
        _ => "All",
    }
}

/// Distributed stack flamegraph — merge identical stacks across ranks.
#[component]
pub fn StackDistributed(mode: String) -> Element {
    let reload = *STACK_DIST_RELOAD.read();
    let cluster = *STACK_DIST_CLUSTER.read();
    let api_mode = if mode == "py" { "py" } else { "mixed" };
    let on_full = api_mode == "mixed";
    let scope = if cluster {
        "cluster fan-out"
    } else {
        "this node"
    };
    let subtitle = if api_mode == "py" {
        format!("Python call paths merged by sample count · {scope}")
    } else {
        format!("Mixed-language call paths merged by sample count · {scope}")
    };

    rsx! {
        PageContainer {
            PageTitle {
                title: "Distributed stacks".to_string(),
                subtitle: Some(subtitle),
                icon: Some(&icondata::AiClusterOutlined),
            }
            div { class: "flex flex-col flex-1 min-h-0 min-w-0 gap-3",
                div {
                    class: "flex gap-1 border-b border-slate-700/80",
                    DistViewTab {
                        label: "Full stack",
                        active: on_full,
                        route: Route::StackDistributedFullPage {},
                    }
                    DistViewTab {
                        label: "Python only",
                        active: !on_full,
                        route: Route::StackDistributedPyPage {},
                    }
                }
                ProfilingContentPanel {
                    AsyncBoundary {
                        message: Some("Loading distributed flamegraph…".to_string()),
                        StackDistributedFlamegraph {
                            key: "stack-dist-{reload}-{cluster}-{api_mode}",
                            cluster,
                            mode: api_mode.to_string(),
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn DistViewTab(label: &'static str, active: bool, route: Route) -> Element {
    let class = if active {
        "px-3 py-2 text-sm font-medium border-b-2 border-blue-500 text-blue-200 -mb-px"
    } else {
        "px-3 py-2 text-sm text-slate-400 hover:text-slate-200 border-b-2 border-transparent -mb-px"
    };
    rsx! {
        Link {
            to: route,
            class: "{class}",
            "{label}"
        }
    }
}

#[component]
fn StackDistributedFlamegraph(cluster: bool, mode: String) -> Element {
    let payload = use_app_resource(move || {
        let mode = mode.clone();
        async move {
            let body = ApiClient::new()
                .get_distributed_stack_flamegraph_json(cluster, &mode)
                .await?;
            let parsed: FlamegraphPayload = serde_json::from_str(&body)
                .map_err(|e| AppError::Api(format!("Invalid flamegraph JSON: {e}")))?;
            Ok(parsed)
        }
    });

    match payload.suspend()?() {
        Ok(data) => {
            let total = data.total;
            let rank_count = data.rank_count;
            let frame_count = data.frames.len();
            let nodes_failed = data.nodes_failed.clone();
            rsx! {
                div { class: "flex min-h-0 flex-1 flex-col",
                    DistributedStackEvidence { total, rank_count, frame_count, nodes_failed }
                    FlamegraphView {
                        payload: data,
                        thread_tid: None,
                        torch_metric: None,
                        on_torch_metric: None,
                    }
                }
            }
        }
        Err(err) => rsx! {
            ProfilingErrorPanel {
                title: "Distributed stack flamegraph".to_string(),
                error: err.display_message(),
            }
        },
    }
}

#[component]
fn DistributedStackEvidence(
    total: u64,
    rank_count: Option<usize>,
    frame_count: usize,
    nodes_failed: Vec<String>,
) -> Element {
    let ranks = rank_count
        .map(|count| count.to_string())
        .unwrap_or_else(|| "—".to_string());
    let failed_title = nodes_failed.join("\n");
    rsx! {
        div { class: "border-b border-gray-200 bg-white px-4 py-3",
            div { class: "grid grid-cols-4 divide-x divide-gray-200",
                StackMetric { label: "Samples", value: total.to_string() }
                StackMetric { label: "Ranks included", value: ranks }
                StackMetric { label: "Merged frames", value: frame_count.to_string() }
                StackMetric { label: "Failed peers", value: nodes_failed.len().to_string() }
            }
            if !nodes_failed.is_empty() {
                p { class: "mt-2 truncate text-[10px] text-amber-700", title: "{failed_title}",
                    "Partial result · {nodes_failed.len()} peer(s) did not contribute"
                }
            }
        }
    }
}
