//! App entry and routing.
//!
//! Each route variant maps to a page component wrapped in [AppLayout](crate::components::layout::AppLayout).
//! See `DESIGN.md` in this directory for structure and conventions.

use dioxus::prelude::*;
use dioxus_router::{Routable, Router};

use crate::components::source_viewer::SourceViewerOverlay;

use crate::components::common::LoadingState;
use crate::components::layout::AppLayout;
use crate::pages::{
    agent::Agent, analytics::Analytics, cluster::Cluster, dashboard::Dashboard,
    profiling::Profiling, pulsing::Pulsing, python::Python, stack::Stack, traces::Traces,
    training::Training,
};
use crate::state::profiling::normalize_profiling_view;

/// All routes. Each is rendered inside AppLayout by the corresponding page component below.
#[derive(Routable, Clone, PartialEq)]
#[rustfmt::skip]
pub enum Route {
    #[route("/")]
    DashboardPage {},
    #[route("/agent")]
    AgentPage {},
    #[route("/cluster")]
    ClusterPage {},
    #[route("/stacks")]
    StackPage {},
    #[route("/stacks/:tid")]
    StackWithTidPage { tid: String },
    #[route("/profiling")]
    ProfilingRedirect {},
    #[route("/profiling/:view")]
    ProfilingViewPage { view: String },
    #[route("/analytics")]
    AnalyticsPage {},
    #[route("/python")]
    PythonPage {},
    #[route("/traces")]
    TracesPage {},
    #[route("/spans")]
    SpansPage {},
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
pub fn AgentPage() -> Element {
    rsx! { AppLayout { Agent {} } }
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
pub fn ProfilingRedirect() -> Element {
    let nav = dioxus_router::use_navigator();
    use_effect(move || {
        nav.replace(Route::ProfilingViewPage {
            view: "pprof".to_string(),
        });
    });
    rsx! {
        AppLayout {
            fullscreen: true,
            LoadingState { message: Some("Opening profiling…".to_string()) }
        }
    }
}

#[component]
pub fn ChromeTracingRedirect() -> Element {
    let nav = dioxus_router::use_navigator();
    use_effect(move || {
        nav.replace(Route::ProfilingViewPage {
            view: "trace".to_string(),
        });
    });
    rsx! {
        AppLayout {
            fullscreen: true,
            LoadingState { message: Some("Opening trace timeline…".to_string()) }
        }
    }
}

#[component]
pub fn ProfilingViewPage(view: String) -> Element {
    let canonical = normalize_profiling_view(&view).to_string();
    if view != canonical {
        return rsx! {
            ProfilingSlugRedirect { target: canonical }
        };
    }

    rsx! {
        AppLayout {
            fullscreen: true,
            Profiling { key: "{canonical}", view: canonical }
        }
    }
}

#[component]
fn ProfilingSlugRedirect(target: String) -> Element {
    let nav = dioxus_router::use_navigator();
    use_effect(move || {
        nav.replace(Route::ProfilingViewPage {
            view: target.clone(),
        });
    });
    rsx! {
        AppLayout {
            fullscreen: true,
            LoadingState { message: Some("Redirecting…".to_string()) }
        }
    }
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
    rsx! {
        AppLayout {
            fullscreen: true,
            Traces {}
        }
    }
}

#[component]
pub fn SpansPage() -> Element {
    rsx! {
        AppLayout {
            fullscreen: true,
            Traces {}
        }
    }
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
        SourceViewerOverlay {}
        Router::<Route> {}
    }
}
