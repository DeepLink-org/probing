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

// 全局状态：Profiling 视图类型
pub static PROFILING_VIEW: GlobalSignal<String> = Signal::global(|| "pprof".to_string());

// Profiling 控制状态
pub static PROFILING_PPROF_FREQ: GlobalSignal<i32> = Signal::global(|| 99);
pub static PROFILING_TORCH_ENABLED: GlobalSignal<bool> = Signal::global(|| false);
pub static PROFILING_CHROME_DATA_SOURCE: GlobalSignal<String> = Signal::global(|| "trace".to_string());
pub static PROFILING_CHROME_LIMIT: GlobalSignal<usize> = Signal::global(|| 1000);
pub static PROFILING_PYTORCH_STEPS: GlobalSignal<i32> = Signal::global(|| 5);

// 侧边栏状态
pub static SIDEBAR_WIDTH: GlobalSignal<f64> = Signal::global(|| 256.0); // 默认 256px (w-64)
pub static SIDEBAR_HIDDEN: GlobalSignal<bool> = Signal::global(|| false);

#[component]
pub fn App() -> Element {
    rsx! {
        Router::<Route> {}
    }
}