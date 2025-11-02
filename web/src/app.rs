use dioxus::prelude::*;
use dioxus_router::{Routable, Router};

use crate::components::layout::AppLayout;
use crate::pages::{
    activity::Activity, cluster::Cluster, overview::Overview, profiler::Profiler,
    python::Python, timeseries::Timeseries,
};

#[derive(Routable, Clone, PartialEq)]
#[rustfmt::skip]
pub enum Route {
    #[route("/")]
    OverviewPage {},
    #[route("/cluster")]
    ClusterPage {},
    #[route("/activity")]
    ActivityPage {},
    #[route("/profiler")]
    ProfilerPage {},
    #[route("/timeseries")]
    TimeseriesPage {},
    #[route("/inspect")]
    PythonPage {},
}

#[component]
pub fn OverviewPage() -> Element {
    rsx! { AppLayout { Overview {} } }
}

#[component]
pub fn ClusterPage() -> Element {
    rsx! { AppLayout { Cluster {} } }
}

#[component]
pub fn ActivityPage() -> Element {
    rsx! { AppLayout { Activity { tid: None } } }
}

#[component]
pub fn ProfilerPage() -> Element {
    rsx! { AppLayout { Profiler {} } }
}

#[component]
pub fn TimeseriesPage() -> Element {
    rsx! { AppLayout { Timeseries {} } }
}

#[component]
pub fn PythonPage() -> Element {
    rsx! { AppLayout { Python {} } }
}

#[component]
pub fn App() -> Element {
    rsx! {
        Router::<Route> {}
    }
}