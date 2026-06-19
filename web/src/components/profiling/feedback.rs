//! Non-blocking feedback toast for profiling SET / actions.

use dioxus::prelude::*;

use crate::components::icon::Icon;
use crate::state::profiling::{clear_profiling_feedback, PROFILING_FEEDBACK};

#[component]
pub fn ProfilingFeedbackToast() -> Element {
    let feedback = PROFILING_FEEDBACK.read().clone();

    let Some(fb) = feedback else {
        return rsx! {};
    };

    let bar = if fb.is_error {
        "fixed bottom-6 right-6 z-[9996] max-w-sm flex items-start gap-2 px-4 py-3 rounded-lg shadow-lg border border-red-200 bg-red-50 text-red-900 text-sm"
    } else {
        "fixed bottom-6 right-6 z-[9996] max-w-sm flex items-start gap-2 px-4 py-3 rounded-lg shadow-lg border border-green-200 bg-green-50 text-green-900 text-sm"
    };
    let icon = if fb.is_error {
        &icondata::AiCloseCircleOutlined
    } else {
        &icondata::AiCheckCircleOutlined
    };

    rsx! {
        div {
            class: "{bar}",
            Icon { icon, class: "w-4 h-4 shrink-0 mt-0.5" }
            span { class: "flex-1", "{fb.message}" }
            button {
                class: "shrink-0 p-0.5 rounded opacity-60 hover:opacity-100",
                onclick: move |_| clear_profiling_feedback(),
                Icon { icon: &icondata::AiCloseOutlined, class: "w-3.5 h-3.5" }
            }
        }
    }
}
