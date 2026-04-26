//! App entry and routing.
//!
//! Each route variant maps to a page component wrapped in [AppLayout](crate::components::layout::AppLayout).
//! See `DESIGN.md` in this directory for structure and conventions.

use dioxus::prelude::*;
use dioxus_router::{Routable, Router};

use crate::components::layout::AppLayout;
use crate::pages::{
    analytics::Analytics, chrome_tracing::ChromeTracing, cluster::Cluster, dashboard::Dashboard,
    profiling::Profiling, pulsing::Pulsing, python::Python, stack::Stack, traces::Traces,
};

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
    #[route("/profiling")]
    ProfilingPage {},
    #[route("/analytics")]
    AnalyticsPage {},
    #[route("/python")]
    PythonPage {},
    #[route("/traces")]
    TracesPage {},
    #[route("/chrome-tracing")]
    ChromeTracingPage {},
    #[route("/pulsing")]
    PulsingPage {},
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
pub fn ChromeTracingPage() -> Element {
    rsx! { AppLayout { ChromeTracing {} } }
}

#[component]
pub fn PulsingPage() -> Element {
    rsx! { AppLayout { Pulsing {} } }
}

#[component]
pub fn App() -> Element {
    rsx! {
        Router::<Route> {}
    }
}
