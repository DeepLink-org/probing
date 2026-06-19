//! App entry and routing.
//!
//! Each route variant maps to a page component wrapped in [AppLayout](crate::components::layout::AppLayout).
//! See `DESIGN.md` in this directory for structure and conventions.

use dioxus::prelude::*;
use dioxus_router::{Routable, Router};

use crate::components::layout::AppLayout;
use crate::pages::{
    analytics::Analytics, cluster::Cluster, dashboard::Dashboard, profiling::Profiling,
    pulsing::Pulsing, python::Python, stack::Stack, traces::Traces, training::Training,
};
use crate::state::profiling::PROFILING_VIEW;

/// All routes. Each is rendered inside AppLayout by the corresponding page component below.
#[derive(Routable, Clone, PartialEq)]
#[rustfmt::skip]
pub enum Route {
    #[route("/")]
    DashboardPage {},
    #[route("/cluster")]
    ClusterPage {},
    #[route("/stacks")]
    StackPage {},
    #[route("/stacks/:tid")]
    StackWithTidPage { tid: String },
    #[route("/profiling")]
    ProfilingPage {},
    #[route("/analytics")]
    AnalyticsPage {},
    #[route("/python")]
    PythonPage {},
    #[route("/traces")]
    TracesPage {},
    #[route("/chrome-tracing")]
    ChromeTracingRedirect {},
    #[route("/pulsing")]
    PulsingPage {},
    #[route("/training")]
    TrainingPage {},
}

// --- Page route components: each wraps a page in AppLayout ---

#[component]
pub fn DashboardPage() -> Element {
    rsx! { AppLayout { Dashboard {} } }
}

#[component]
pub fn ClusterPage() -> Element {
    rsx! { AppLayout { Cluster {} } }
}

#[component]
pub fn StackPage() -> Element {
    rsx! { AppLayout { Stack { tid: None } } }
}

#[component]
pub fn StackWithTidPage(tid: String) -> Element {
    rsx! { AppLayout { Stack { tid: Some(tid) } } }
}

#[component]
pub fn ChromeTracingRedirect() -> Element {
    let nav = dioxus_router::use_navigator();
    use_effect(move || {
        *PROFILING_VIEW.write() = "trace-timeline".to_string();
        nav.replace(Route::ProfilingPage {});
    });
    rsx! {
        AppLayout {
            crate::components::common::LoadingState {
                message: Some("Opening trace timeline…".to_string()),
            }
        }
    }
}

#[component]
pub fn ProfilingPage() -> Element {
    rsx! { AppLayout { Profiling {} } }
}

#[component]
pub fn AnalyticsPage() -> Element {
    rsx! { AppLayout { Analytics {} } }
}

#[component]
pub fn PythonPage() -> Element {
    rsx! { AppLayout { Python {} } }
}

#[component]
pub fn TracesPage() -> Element {
    rsx! { AppLayout { Traces {} } }
}

#[component]
pub fn PulsingPage() -> Element {
    rsx! { AppLayout { Pulsing {} } }
}

#[component]
pub fn TrainingPage() -> Element {
    rsx! { AppLayout { Training {} } }
}

#[component]
pub fn App() -> Element {
    rsx! {
        Router::<Route> {}
    }
}
