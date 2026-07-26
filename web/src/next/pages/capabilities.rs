//! Route-neutral mounts for mature Classic capabilities.
//!
//! These wrappers deliberately reuse the proven page implementations while the
//! Next shell owns navigation, global tools, and information architecture.

use dioxus::prelude::*;

use crate::pages::rl::RlViewMode;

use super::super::components::InlineNotice;

#[component]
pub fn SystemPage() -> Element {
    rsx! {
        div { class: "space-y-4",
            MigrationNotice {
                detail: "Detailed CPU/GPU trends, process metadata, threads, environment variables, and thread-to-evidence links are available here."
            }
            crate::pages::dashboard::Dashboard {}
        }
    }
}

#[component]
pub fn AnalyticsPage() -> Element {
    rsx! { crate::pages::analytics::Analytics {} }
}

#[component]
pub fn ClusterPage() -> Element {
    rsx! { crate::pages::cluster::Cluster {} }
}

#[component]
pub fn SpansPage() -> Element {
    rsx! { crate::pages::traces::Traces { show_context_controls: false } }
}

#[component]
pub fn PythonPage() -> Element {
    rsx! { crate::pages::python::Python {} }
}

#[component]
pub fn PulsingPage() -> Element {
    rsx! { crate::pages::pulsing::Pulsing {} }
}

#[component]
pub fn StackPage() -> Element {
    rsx! { crate::pages::stack::Stack { tid: None } }
}

#[component]
pub fn StackThreadPage(tid: String) -> Element {
    rsx! { crate::pages::stack::Stack { tid: Some(tid) } }
}

#[component]
pub fn DistributedStackPage() -> Element {
    rsx! { crate::pages::stack::StackDistributed { mode: "mixed".to_string() } }
}

#[component]
pub fn DistributedPythonStackPage() -> Element {
    rsx! { crate::pages::stack::StackDistributed { mode: "py".to_string() } }
}

#[component]
pub fn RolloutPage() -> Element {
    rsx! { crate::pages::rl::RlObservability {
        view: RlViewMode::Rollout,
        show_context_controls: false,
    } }
}

#[component]
pub fn RlTrainPage() -> Element {
    rsx! { crate::pages::rl::RlObservability {
        view: RlViewMode::Train,
        show_context_controls: false,
    } }
}

#[component]
pub fn RlSpansPage() -> Element {
    rsx! { crate::pages::rl::RlObservability {
        view: RlViewMode::Spans,
        show_context_controls: false,
    } }
}

#[component]
pub fn ProcessTimelinePage() -> Element {
    rsx! { crate::pages::rl::RlObservability {
        view: RlViewMode::ProcessTimeline,
        show_context_controls: false,
    } }
}

#[component]
pub fn PerfettoPage() -> Element {
    rsx! {
        div { class: "h-[calc(100vh-8rem)] min-h-[36rem] overflow-hidden rounded-xl border border-gray-200 bg-white shadow-sm",
            crate::pages::rl::RlObservability {
                view: RlViewMode::Perfetto,
                show_context_controls: false,
            }
        }
    }
}

#[component]
pub fn InferencePage() -> Element {
    rsx! { crate::pages::rl::Inference { show_controls: false } }
}

#[component]
fn MigrationNotice(detail: &'static str) -> Element {
    rsx! {
        InlineNotice {
            title: "Capability parity".to_string(),
            detail: detail.to_string(),
        }
    }
}
