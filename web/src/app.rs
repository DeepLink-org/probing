use dioxus::prelude::*;
use dioxus_router::{Routable, Router};

use crate::components::layout::AppLayout;
use crate::pages::{
    analytics::Analytics, chrome_tracing::ChromeTracing, cluster::Cluster, dashboard::Dashboard, 
    profiling::Profiling, python::Python, stack::Stack, traces::Traces,
};

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
}

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
pub fn App() -> Element {
    rsx! {
        Router::<Route> {}
    }
}