use dioxus::prelude::*;

use crate::state::profiling::normalize_profiling_view;

use super::super::components::NextPageHeader;

#[component]
pub fn ProfilesPage() -> Element {
    rsx! { ProfilesWorkspace { view: "pprof".to_string() } }
}

#[component]
pub fn ProfileViewPage(view: String) -> Element {
    rsx! { ProfilesWorkspace { view } }
}

#[component]
pub fn ChromeTracePage() -> Element {
    rsx! { ProfilesWorkspace { view: "trace".to_string() } }
}

#[component]
fn ProfilesWorkspace(view: String) -> Element {
    let current = normalize_profiling_view(&view).to_string();
    rsx! {
        div { class: "flex min-h-[calc(100vh-8rem)] flex-col gap-4",
            NextPageHeader {
                title: "Performance profiles".to_string(),
                subtitle: "One evidence workspace for CPU sampling, Torch modules, trace events, PyTorch profiler, and Ray timelines.".to_string(),
            }
            div { class: "flex min-h-[36rem] flex-1 flex-col rounded-xl border border-gray-200 bg-white p-4 shadow-sm",
                crate::pages::profiling::Profiling { view: current }
            }
        }
    }
}
