use dioxus::prelude::*;

use crate::components::icon::Icon;
use crate::state::investigation::{
    clear_investigation_context, INVESTIGATION_CONTEXT,
};

/// Slim context strip above main content. Correlate shortcuts live in the sidebar.
#[component]
pub fn InvestigationContextBar() -> Element {
    let ctx = INVESTIGATION_CONTEXT.read().clone();
    if ctx.is_empty() {
        return rsx! {
            div {
                class: "flex flex-wrap items-center gap-1 px-4 py-1.5 border-b border-gray-100 bg-gray-50/80 text-[11px] text-gray-500",
                span {
                    title: "Set span or trace context from Training, Spans, or Investigate — use Correlate in the sidebar",
                    "Investigation context empty — set context from Training or Spans"
                }
            }
        };
    }

    rsx! {
        div {
            class: "flex flex-wrap items-center gap-2 px-4 py-1.5 border-b border-blue-100 bg-blue-50/80 text-xs",
            span {
                class: "inline-flex items-center gap-1 text-blue-800 font-medium shrink-0",
                Icon { icon: &icondata::AiBulbOutlined, class: "w-3.5 h-3.5" }
                "Context"
            }
            span {
                class: "text-blue-900 font-mono truncate max-w-[40rem]",
                title: "{ctx.summary()}",
                "{ctx.summary()}"
            }
            button {
                class: "ml-auto shrink-0 px-2 py-0.5 rounded-md border border-blue-200 bg-white text-blue-700 hover:bg-blue-100/60 text-[11px]",
                onclick: move |_| clear_investigation_context(),
                "Clear"
            }
        }
    }
}
