//! Inline hint when a page should reflect the global investigation context.

use dioxus::prelude::*;
use dioxus_router::Link;

use crate::app::Route;
use crate::state::investigation::INVESTIGATION_CONTEXT;

#[component]
pub fn InvestigationContextHint(
    #[props(default = "Process-wide view.")] note: &'static str,
) -> Element {
    let ctx = INVESTIGATION_CONTEXT.read().clone();
    if ctx.trace_id.is_none() && ctx.span_name.is_none() && ctx.pid.is_none() {
        return rsx! {};
    }

    rsx! {
        div {
            class: "mb-3 flex flex-wrap items-center gap-x-2 gap-y-1 px-3 py-2 text-xs rounded-md border border-blue-100 bg-blue-50/80 text-blue-900",
            span { class: "font-medium shrink-0", "Investigation context" }
            span { class: "font-mono truncate max-w-md", "{ctx.summary()}" }
            if ctx.trace_id.is_some() || ctx.span_name.is_some() {
                Link {
                    to: Route::SpansPage {},
                    class: "shrink-0 px-2 py-0.5 rounded border border-blue-200 bg-white text-blue-700 hover:bg-blue-50",
                    "Spans"
                }
            }
            span { class: "text-blue-700/90", "{note}" }
        }
    }
}
