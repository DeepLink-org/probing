use std::collections::HashMap;

use dioxus::prelude::*;

use super::logic::{
    ancestor_ids, child_map, descendants, format_frame_value, format_pct, frame_fill_color,
    frame_matches_thread_tid, frame_visible_for_thread, index_frames, is_torch_profile,
    label_for_frame, leaf_count_label, list_phases, matches_search, metric_value_label,
    phase_label, search_placeholder, TORCH_METRICS,
};
use super::model::{FlameFrame, FlamegraphPayload};
use super::widgets::{
    BreadcrumbBar, BreadcrumbLink, BreadcrumbSeparator, ChartPanel, FlamegraphShell, FlamegraphSvg,
    FlamegraphToolbar, FloatingTooltip, MetricPill, MetricPillRow, PhasePill, PhasePillRow,
    PhasePillTone, StackSearchInput, StatChip, StatChipRow, TooltipState,
};

#[component]
pub fn StackExplorerView(
    payload: FlamegraphPayload,
    #[props(optional)] torch_metric: Option<Signal<String>>,
    #[props(optional)] on_torch_metric: Option<EventHandler<String>>,
    #[props(optional)] thread_tid: Option<i32>,
) -> Element {
    let profile = payload.profile.clone();
    let count_name = payload.count_name.clone();
    let payload_metric = payload.metric.clone();
    let torch = is_torch_profile(&profile);
    let frames = payload.frames.clone();
    let root_id = frames.first().map(|f| f.id).unwrap_or(0);
    let by_id = index_frames(&frames);
    let children = child_map(&frames);
    let phases = if torch {
        list_phases(&frames)
    } else {
        vec!["all".to_string()]
    };
    let frame_height = payload.frame_height;
    let graph_width = payload.width;
    let total = payload.total;
    let search_ph = search_placeholder(&profile);
    let leaf_label = leaf_count_label(&profile);

    let mut zoom_id = use_signal(|| root_id);
    let mut phase_filter = use_signal(|| "all".to_string());
    let mut search_query = use_signal(String::new);
    let mut tooltip = use_signal(|| None::<TooltipState>);

    use_effect({
        let frames = frames.clone();
        let torch_profile = torch;
        move || {
            if let Some(tid) = thread_tid {
                if !torch_profile {
                    if let Some(tf) = frames
                        .iter()
                        .find(|f| f.depth == 1 && frame_matches_thread_tid(&f.name, tid))
                    {
                        zoom_id.set(tf.id);
                    }
                }
            }
        }
    });

    let active_metric = if torch {
        torch_metric
            .map(|s| s())
            .or_else(|| payload_metric.clone())
            .unwrap_or_else(|| "duration".to_string())
    } else {
        "duration".to_string()
    };
    let zoom_set = descendants(&children, *zoom_id.read());
    let root = by_id.get(&*zoom_id.read()).cloned();
    let query = search_query.read().trim().to_string();
    let any_search = !query.is_empty();
    let phase_active = phase_filter.read().clone();

    let visible: Vec<FlameFrame> = frames
        .iter()
        .filter(|f| zoom_set.contains(&f.id))
        .filter(|f| {
            if !torch || phase_active == "all" {
                true
            } else {
                f.depth <= 1 || f.phase.as_deref() == Some(phase_active.as_str())
            }
        })
        .filter(|f| thread_tid.is_none_or(|tid| frame_visible_for_thread(&by_id, f, tid)))
        .cloned()
        .collect();

    let max_depth = visible.iter().map(|f| f.depth).max().unwrap_or(0);
    let svg_height = (max_depth + 1) as f64 * frame_height + 12.0;
    let leaf_count = if torch {
        visible.iter().filter(|f| f.depth >= 2).count()
    } else {
        visible.iter().filter(|f| f.depth > 0).count()
    };
    let scope = root.as_ref().map(|r| r.value).unwrap_or(0);
    let scope_label = format_frame_value(scope, &count_name);
    let scope_pct = format_pct(scope, total);
    let ancestor_chain = ancestor_ids(&by_id, *zoom_id.read());
    let phase_roots: HashMap<String, usize> = frames
        .iter()
        .filter(|f| f.depth == 1)
        .map(|f| (f.name.clone(), f.id))
        .collect();

    let selected_ranks = root.as_ref().map(|r| r.ranks.clone()).unwrap_or_default();
    let is_distributed = profile.contains("distributed");
    let rank_count = payload.rank_count;

    let reset_view = move |_| {
        zoom_id.set(root_id);
        phase_filter.set("all".to_string());
        tooltip.set(None);
    };

    rsx! {
        FlamegraphShell {
            ExplorerToolbar {
                torch,
                metric: active_metric.clone(),
                phases,
                phase_filter: phase_active.clone(),
                search_query: (*search_query.read()).clone(),
                search_ph,
                scope_label,
                scope_pct,
                leaf_count,
                leaf_label,
                on_search: EventHandler::new(move |value: String| search_query.set(value)),
                on_phase: EventHandler::new(move |phase: String| {
                    phase_filter.set(phase.clone());
                    if phase == "all" {
                        zoom_id.set(root_id);
                    } else if let Some(id) = phase_roots.get(&phase) {
                        zoom_id.set(*id);
                    }
                }),
                on_metric: on_torch_metric,
            }
            if is_distributed {
                RankPartitionPanel {
                    ranks: selected_ranks,
                    rank_count,
                    frame_name: root.as_ref().map(label_for_frame).unwrap_or_default(),
                }
            }
            ExplorerBreadcrumbs {
                ancestor_chain,
                by_id,
                on_select: EventHandler::new(move |id: usize| zoom_id.set(id)),
            }
            ChartPanel {
                onclick: EventHandler::new(reset_view),
                FlamegraphSvg {
                    width: graph_width,
                    height: svg_height,
                    for frame in visible.iter().filter(|f| f.depth > 0) {
                        ExplorerFrameNode {
                            profile: profile.clone(),
                            count_name: count_name.clone(),
                            metric: payload_metric.clone(),
                            frame: frame.clone(),
                            root: root.clone(),
                            frame_height,
                            graph_width,
                            total,
                            any_search,
                            query: query.clone(),
                            zoom_id,
                            tooltip,
                        }
                    }
                }
            }
            if let Some(tip) = tooltip.read().clone() {
                FloatingTooltip { state: tip }
            }
        }
    }
}

#[component]
fn RankPartitionPanel(ranks: Vec<i32>, rank_count: Option<usize>, frame_name: String) -> Element {
    let ranks_label = if ranks.is_empty() {
        "unknown".to_string()
    } else {
        ranks
            .iter()
            .map(|r| r.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let coverage = match rank_count {
        Some(n) if n > 0 && !ranks.is_empty() => {
            format!(" · {}/{} ranks", ranks.len(), n)
        }
        _ if !ranks.is_empty() => format!(" · {} ranks", ranks.len()),
        _ => String::new(),
    };
    let title = if frame_name.is_empty() || frame_name == "all" {
        "Selected partition".to_string()
    } else {
        format!("Selected: {frame_name}")
    };

    rsx! {
        div {
            class: "px-4 py-2.5 border-b border-slate-200 bg-slate-50 flex flex-wrap items-start gap-2",
            div { class: "min-w-0 flex-1",
                p { class: "text-[10px] uppercase tracking-wide text-slate-500 mb-1", "{title}" }
                p { class: "text-xs text-slate-600",
                    span { class: "text-slate-500", "Ranks in this fork{coverage}: " }
                    span { class: "font-mono text-slate-800 font-medium", "{ranks_label}" }
                }
            }
            if !ranks.is_empty() {
                div { class: "flex flex-wrap gap-1",
                    for r in ranks.iter() {
                        span {
                            class: "inline-flex items-center px-1.5 py-0.5 rounded text-[11px] font-mono bg-white border border-slate-200 text-slate-700",
                            "rank {r}"
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn ExplorerToolbar(
    torch: bool,
    metric: String,
    phases: Vec<String>,
    phase_filter: String,
    search_query: String,
    search_ph: &'static str,
    scope_label: String,
    scope_pct: String,
    leaf_count: usize,
    leaf_label: &'static str,
    on_search: EventHandler<String>,
    on_phase: EventHandler<String>,
    #[props(optional)] on_metric: Option<EventHandler<String>>,
) -> Element {
    rsx! {
        FlamegraphToolbar {
            StackSearchInput {
                value: search_query,
                placeholder: search_ph,
                on_input: on_search,
            }
            if torch {
                if let Some(on_metric) = on_metric {
                    MetricPillRow {
                        for (id, label) in TORCH_METRICS.iter() {
                            {
                                let id = id.to_string();
                                let label = label.to_string();
                                let active = metric == id;
                                rsx! {
                                    MetricPill {
                                        label,
                                        active,
                                        onclick: EventHandler::new(move |_| on_metric.call(id.clone())),
                                    }
                                }
                            }
                        }
                    }
                }
                PhasePillRow {
                    for phase in phases.iter() {
                        {
                            let phase = phase.clone();
                            let active = phase_filter == phase;
                            let tone = PhasePillTone::from_phase(&phase);
                            let label = phase_label(&phase).to_string();
                            rsx! {
                                PhasePill {
                                    label,
                                    active,
                                    tone,
                                    onclick: EventHandler::new(move |_| on_phase.call(phase.clone())),
                                }
                            }
                        }
                    }
                }
            }
            StatChipRow {
                StatChip { label: "View", value: scope_label }
                StatChip { label: "Share", value: format!("{scope_pct}%") }
                StatChip { label: leaf_label, value: leaf_count.to_string() }
            }
        }
    }
}

#[component]
fn ExplorerBreadcrumbs(
    ancestor_chain: Vec<usize>,
    by_id: HashMap<usize, FlameFrame>,
    on_select: EventHandler<usize>,
) -> Element {
    rsx! {
        BreadcrumbBar {
            for (i, id) in ancestor_chain.iter().enumerate() {
                {
                    let frame = by_id.get(id).cloned();
                    if let Some(f) = frame {
                        let label = label_for_frame(&f);
                        let frame_id = f.id;
                        rsx! {
                            if i > 0 {
                                BreadcrumbSeparator {}
                            }
                            BreadcrumbLink {
                                label,
                                onclick: EventHandler::new(move |_| on_select.call(frame_id)),
                            }
                        }
                    } else {
                        rsx! {}
                    }
                }
            }
        }
    }
}

#[component]
fn ExplorerFrameNode(
    profile: String,
    count_name: String,
    metric: Option<String>,
    frame: FlameFrame,
    root: Option<FlameFrame>,
    frame_height: f64,
    graph_width: f64,
    total: u64,
    any_search: bool,
    query: String,
    zoom_id: Signal<usize>,
    tooltip: Signal<Option<TooltipState>>,
) -> Element {
    let root_frame = match root {
        Some(r) => r,
        None => return rsx! {},
    };

    let rx = ((frame.x - root_frame.x) / root_frame.w) * graph_width;
    let rw = (frame.w / root_frame.w) * graph_width;
    if rw < 1.0 {
        return rsx! {};
    }

    let torch = is_torch_profile(&profile);
    let fill = frame_fill_color(&profile, &frame);
    let y = frame.depth as f64 * frame_height;
    let pad = 2.0;
    let matched = matches_search(&frame, &query);
    let opacity = if any_search && !matched { "0.22" } else { "1" };
    let label = if torch && frame.depth >= 2 {
        frame.name.clone()
    } else {
        label_for_frame(&frame)
    };
    let show_label = rw > 48.0 && frame.depth >= 1;
    let max_chars = ((rw - 16.0) / 7.0).floor() as usize;
    let display_label = if label.len() > max_chars {
        format!("{}…", label.chars().take(max_chars).collect::<String>())
    } else {
        label.clone()
    };

    let frame_id = frame.id;
    let frame_value = frame.value;
    let phase_str = frame.phase.as_deref().unwrap_or("other").to_string();
    let root_value = root_frame.value;
    let path_prefix = if torch { "Module" } else { "Stack" };

    rsx! {
        g {
            class: "cursor-pointer",
            opacity: "{opacity}",
            onclick: move |evt| {
                evt.stop_propagation();
                zoom_id.set(frame_id);
                tooltip.set(None);
            },
            onmouseenter: move |evt: Event<MouseData>| {
                let coords = evt.data().client_coordinates();
                let value_label = metric_value_label(metric.as_deref(), &count_name);
                let mut lines = vec![
                    format!(
                        "{value_label}: {}",
                        format_frame_value(frame_value, &count_name)
                    ),
                    format!(
                        "Of view: {}% · Of total: {}%",
                        format_pct(frame_value, root_value),
                        format_pct(frame_value, total),
                    ),
                ];
                if !frame.ranks.is_empty() {
                    let ranks = frame
                        .ranks
                        .iter()
                        .map(|r| r.to_string())
                        .collect::<Vec<_>>()
                        .join(", ");
                    lines.push(format!("Ranks: {ranks}"));
                }
                if let Some(path) = &frame.module_path {
                    lines.push(format!("{path_prefix}: {path}"));
                }
                if torch {
                    lines.push(format!("Phase: {phase_str}"));
                }
                tooltip.set(Some(TooltipState {
                    title: label_for_frame(&frame),
                    lines,
                    x: coords.x,
                    y: coords.y,
                }));
            },
            onmouseleave: move |_| tooltip.set(None),
            rect {
                x: "{rx + pad}",
                y: "{y + pad}",
                width: "{rw - pad * 2.0}",
                height: "{frame_height - pad * 2.0 - 2.0}",
                rx: "6",
                fill: "{fill}",
                stroke: if any_search && matched { "white" } else { "rgba(255,255,255,0.08)" },
                stroke_width: "1",
            }
            if show_label {
                text {
                    x: "{rx + 10.0}",
                    y: "{y + frame_height * 0.62}",
                    fill: "#f8fafc",
                    font_size: "12",
                    font_weight: if frame.depth == 1 { "600" } else { "500" },
                    "{display_label}"
                }
            }
        }
    }
}
