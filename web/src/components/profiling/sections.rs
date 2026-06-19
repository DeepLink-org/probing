//! Layout shells for the Profiling page.

use dioxus::prelude::*;

use crate::components::common::{EmptyState, ErrorState};

#[component]
pub fn ProfilingContentPanel(children: Element) -> Element {
    rsx! {
        div {
            class: "bg-white rounded-lg border border-gray-200 overflow-hidden",
            style: "min-height: calc(100vh - 12rem);",
            {children}
        }
    }
}

#[component]
pub fn ProfilingCenteredPanel(children: Element) -> Element {
    rsx! {
        div {
            class: "flex items-center justify-center py-24 p-8",
            div { class: "text-center", {children} }
        }
    }
}

#[component]
pub fn ProfilerDisabledNotice(profiler_name: &'static str) -> Element {
    let message = format!(
        "No profilers are currently enabled. Enable {profiler_name} using the controls in the sidebar."
    );
    rsx! {
        ProfilingCenteredPanel {
            h2 {
                class: "text-2xl font-bold text-gray-900 mb-4",
                "No Profilers Enabled"
            }
            EmptyState { message }
        }
    }
}

#[component]
pub fn ProfilingErrorPanel(title: String, error: String) -> Element {
    rsx! {
        div { class: "p-6",
            ErrorState { error, title: Some(title) }
        }
    }
}

#[component]
pub fn TimelinePlaceholder(title: &'static str, hint: String) -> Element {
    rsx! {
        ProfilingCenteredPanel {
            div { class: "text-center text-gray-500",
                p { class: "mb-4 text-lg", "{title}" }
                p { class: "text-sm", "{hint}" }
            }
        }
    }
}

#[component]
pub fn TimelinePanel(children: Element) -> Element {
    rsx! {
        div { class: "relative min-h-[600px]", {children} }
    }
}
