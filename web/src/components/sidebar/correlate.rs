//! Investigation correlate shortcuts — lives in the sidebar to avoid eating main content space.

use dioxus::prelude::*;
use dioxus_router::{use_navigator, use_route};

use crate::app::Route;
use crate::components::colors::colors;
use crate::components::icon::Icon;
use crate::state::investigation::{
    clear_investigation_context, INVESTIGATION_CONTEXT, InvestigationContext,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum CorrelateKind {
    Dashboard,
    Spans,
    Profile,
    Training,
}

fn correlate_is_current(route: &Route, kind: CorrelateKind) -> bool {
    match kind {
        CorrelateKind::Dashboard => matches!(route, Route::DashboardPage {}),
        CorrelateKind::Spans => matches!(route, Route::SpansPage {} | Route::TracesPage {}),
        CorrelateKind::Profile => matches!(route, Route::ProfilingViewPage { .. }),
        CorrelateKind::Training => matches!(route, Route::TrainingPage {}),
    }
}

fn context_chip_parts(ctx: &InvestigationContext) -> Vec<String> {
    let mut chips = Vec::new();
    if let Some(label) = &ctx.label {
        for part in label.split('·') {
            let text = part.trim();
            if !text.is_empty() {
                chips.push(text.to_string());
            }
        }
    }
    if let Some(name) = &ctx.span_name {
        let tag = format!("span:{name}");
        if !chips.iter().any(|c| c.contains(name.as_str())) {
            chips.push(tag);
        }
    }
    if let Some(trace_id) = ctx.trace_id {
        chips.push(format!("trace {trace_id}"));
    }
    if let Some(pid) = ctx.pid {
        chips.push(format!("pid {pid}"));
    }
    if let Some(tid) = ctx.tid {
        chips.push(format!("tid {tid}"));
    }
    chips
}

/// Sidebar panel: context summary + correlated view shortcuts.
#[component]
pub fn SidebarCorrelatePanel() -> Element {
    let ctx = INVESTIGATION_CONTEXT.read().clone();
    if ctx.is_empty() {
        return rsx! {};
    }

    let chips = context_chip_parts(&ctx);
    let summary = ctx.summary();

    rsx! {
        div {
            class: "px-2 {colors::SIDEBAR_PANEL_BORDER}",
            div { class: "{colors::CORRELATE_PANEL}",
                div { class: "{colors::CORRELATE_PANEL_HEADER}",
                    div { class: "flex items-start gap-2 min-w-0",
                        Icon {
                            icon: &icondata::AiPushpinOutlined,
                            class: "w-3.5 h-3.5 text-amber-400 shrink-0 mt-0.5",
                        }
                        div { class: "flex-1 min-w-0",
                            div { class: "{colors::CORRELATE_PANEL_TITLE}", "Pinned context" }
                            p { class: "{colors::CORRELATE_PANEL_SUBTITLE}",
                                "Jump to a related view — filters carry over"
                            }
                        }
                    }
                    if chips.is_empty() {
                        p {
                            class: "mt-2 text-[11px] text-slate-400 leading-snug line-clamp-2",
                            title: "{summary}",
                            "{summary}"
                        }
                    } else {
                        div { class: "mt-2 flex flex-wrap gap-1",
                            for chip in chips {
                                span { class: "{colors::CORRELATE_CHIP}", "{chip}" }
                            }
                        }
                    }
                    button {
                        class: "{colors::CORRELATE_CLEAR_BTN}",
                        onclick: move |_| clear_investigation_context(),
                        "Clear pinned context"
                    }
                }
                CorrelateLinks { ctx }
            }
        }
    }
}

#[component]
fn CorrelateLinks(ctx: InvestigationContext) -> Element {
    let nav = use_navigator();
    let route = use_route::<Route>();

    rsx! {
        div {
            if ctx.trace_id.is_some() || ctx.span_name.is_some() {
                CorrelateAction {
                    label: "Distributed Spans",
                    hint: "Span tree with current filters",
                    icon: &icondata::AiApiOutlined,
                    is_here: correlate_is_current(&route, CorrelateKind::Spans),
                    onclick: move |_| {
                        nav.push(Route::SpansPage {});
                    },
                }
            }
            if ctx.pid.is_some() {
                CorrelateAction {
                    label: "CPU / Torch Profile",
                    hint: "Flamegraph for this process",
                    icon: &icondata::CgPerformance,
                    is_here: correlate_is_current(&route, CorrelateKind::Profile),
                    onclick: move |_| {
                        nav.push(Route::ProfilingViewPage {
                            view: "pprof".to_string(),
                        });
                    },
                }
            }
            CorrelateAction {
                label: "Live metrics",
                hint: "CPU, GPU, thread ranking",
                icon: &icondata::AiLineChartOutlined,
                is_here: correlate_is_current(&route, CorrelateKind::Dashboard),
                onclick: move |_| {
                    nav.push(Route::DashboardPage {});
                },
            }
            CorrelateAction {
                label: "Training breakdown",
                hint: "Step timing and module hotspots",
                icon: &icondata::AiRadarChartOutlined,
                is_here: correlate_is_current(&route, CorrelateKind::Training),
                onclick: move |_| {
                    nav.push(Route::TrainingPage {});
                },
            }
        }
    }
}

#[component]
fn CorrelateAction(
    label: &'static str,
    hint: &'static str,
    icon: &'static icondata::Icon,
    is_here: bool,
    onclick: EventHandler<()>,
) -> Element {
    let row_class = if is_here {
        colors::CORRELATE_LINK_HERE
    } else {
        colors::CORRELATE_LINK
    };

    rsx! {
        button {
            class: "{row_class}",
            title: "{hint}",
            onclick: move |_| onclick.call(()),
            Icon { icon, class: "w-3.5 h-3.5 shrink-0 opacity-70" }
            span { class: "flex-1 min-w-0",
                div { class: "font-medium truncate", "{label}" }
                div { class: "{colors::CORRELATE_LINK_HINT}", "{hint}" }
            }
            if is_here {
                span {
                    class: "shrink-0 text-[9px] uppercase tracking-wide font-semibold text-amber-400/90",
                    "here"
                }
            } else {
                Icon {
                    icon: &icondata::AiArrowRightOutlined,
                    class: "w-3 h-3 shrink-0 opacity-0 group-hover:opacity-60 text-amber-300/80 transition-opacity",
                }
            }
        }
    }
}
