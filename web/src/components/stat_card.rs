//! Summary metric card (label + large value).

use dioxus::prelude::*;

use crate::components::colors::colors;

#[component]
pub fn StatCard(
    label: &'static str,
    value: String,
    #[props(optional)] hint: Option<String>,
) -> Element {
    rsx! {
        div {
            class: "bg-white border border-gray-200 rounded-lg px-5 py-4 shadow-sm",
            p { class: "text-xs font-medium text-gray-500 uppercase tracking-wide", "{label}" }
            p { class: format!("text-2xl font-bold text-{} mt-1", colors::PRIMARY), "{value}" }
            if let Some(h) = hint {
                p { class: "text-xs text-gray-400 mt-1", "{h}" }
            }
        }
    }
}
