use dioxus::prelude::*;
use dioxus_router::{Link, use_navigator};

use crate::app::Route;
use crate::components::colors::colors;
use crate::components::icon::Icon;
use crate::state::investigation::{
    clear_investigation_context, INVESTIGATION_CONTEXT, InvestigationContext,
};

#[component]
pub fn InvestigationContextBar() -> Element {
    let ctx = INVESTIGATION_CONTEXT.read().clone();
    if ctx.is_empty() {
        return rsx! {
            div {
                class: "px-4 py-1.5 border-b border-gray-100 bg-gray-50/80 text-[11px] text-gray-500",
                span {
                    title: "Select a thread on Dashboard to link Stack, Distributed Spans, and Profile views",
                    "Investigation context empty — pick a thread on Dashboard to link Stack, Spans, and Profile."
                }
            }
        };
    }

    rsx! {
        div {
            class: "flex flex-wrap items-center gap-2 px-4 py-2 bg-blue-50/80 border-b border-blue-100 text-xs",
            span {
                class: "inline-flex items-center gap-1 text-blue-800 font-medium shrink-0",
                Icon { icon: &icondata::AiBulbOutlined, class: "w-3.5 h-3.5" }
                "Context"
            }
            span { class: "text-blue-900 font-mono truncate max-w-[28rem]", "{ctx.summary()}" }
            ContextQuickLinks { ctx: ctx.clone() }
            button {
                class: "ml-auto shrink-0 px-2 py-1 rounded-md border border-blue-200 bg-white text-blue-700 hover:bg-blue-100/60",
                onclick: move |_| clear_investigation_context(),
                "Clear"
            }
        }
    }
}

#[component]
fn ContextQuickLinks(ctx: InvestigationContext) -> Element {
    let nav = use_navigator();
    rsx! {
        div { class: "flex flex-wrap items-center gap-1.5",
            if let Some(tid) = ctx.tid {
                {
                    let tid_str = tid.to_string();
                    rsx! {
                        Link {
                            to: Route::StackWithTidPage { tid: tid_str.clone() },
                            class: "px-2 py-0.5 rounded-md border border-blue-200 bg-white text-blue-700 hover:bg-blue-50",
                            "Stack"
                        }
                    }
                }
                button {
                    class: "px-2 py-0.5 rounded-md border border-blue-200 bg-white text-blue-700 hover:bg-blue-50",
                    title: "Open Distributed Spans filtered to this thread",
                    onclick: move |_| { nav.push(Route::TracesPage {}); },
                    "Spans"
                }
                button {
                    class: format!(
                        "px-2 py-0.5 rounded-md border border-{} bg-{} text-{} hover:opacity-90",
                        colors::CONTENT_ACCENT_BORDER,
                        colors::CONTENT_ACCENT_BG,
                        colors::CONTENT_ACCENT_TEXT,
                    ),
                    onclick: move |_| { nav.push(Route::ProfilingViewPage { view: "pprof".to_string() }); },
                    "Profile"
                }
            }
            if ctx.trace_id.is_some() {
                button {
                    class: "px-2 py-0.5 rounded-md border border-blue-200 bg-white text-blue-700 hover:bg-blue-50",
                    onclick: move |_| { nav.push(Route::TracesPage {}); },
                    "Open spans"
                }
            }
        }
    }
}
