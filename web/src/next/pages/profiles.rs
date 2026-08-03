use dioxus::prelude::*;

use crate::api::ApiClient;
use crate::components::common::AsyncBoundary;
use crate::components::flamegraph::{FlamegraphPayload, FlamegraphView};
use crate::components::profile_snapshot_bar::ProfileSnapshotBar;
use crate::components::profiling::{
    ProfilerDisabledNotice, ProfilingErrorPanel, ProfilingFeedbackToast,
    PytorchChromeTimelineLoader, RayChromeTimelineLoader, TimelinePlaceholder,
    TraceChromeTimelineLoader,
};
use crate::hooks::use_app_resource;
use crate::state::investigation::{
    clear_profiling_thread_filter, INVESTIGATION_CONTEXT, PROFILING_THREAD_FILTER,
};
use crate::state::profiling::{
    apply_profiler_config, normalize_profiling_view, profiling_view_spec, PROFILING_CHROME_LIMIT,
    PROFILING_CONFIG_LOADED, PROFILING_PPROF_FREQ, PROFILING_PYTORCH_TIMELINE_RELOAD,
    PROFILING_RAY_TIMELINE_RELOAD, PROFILING_TORCH_ENABLED, PROFILING_TRACE_RELOAD,
};

use super::super::components::WorkspacePage;

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
    let spec = profiling_view_spec(&current);
    rsx! {
        ProfilingFeedbackToast {}
        WorkspacePage {
            title: spec.label.to_string(),
            subtitle: view_subtitle(&current).to_string(),
            fill: true,
            div { class: "min-h-0 min-w-0 flex-1 overflow-hidden rounded-xl border border-gray-200 bg-white shadow-sm",
                AsyncBoundary {
                    message: Some("Loading profiler configuration".to_string()),
                    ProfilerConfigGate { key: "{current}", view: current }
                }
            }
        }
    }
}

fn view_subtitle(view: &str) -> &'static str {
    match view {
        "pprof" => "SIGPROF statistical samples rendered as a call hierarchy.",
        "torch" => "Measured module hooks rendered as a flamegraph.",
        "trace" => "Chrome trace events from probing buffers, separate from distributed spans.",
        "pytorch" => "PyTorch profiler Chrome trace capture.",
        "ray" => "Ray task timeline capture.",
        _ => "Profiler evidence.",
    }
}

#[component]
fn ProfilerConfigGate(view: String) -> Element {
    let trace_reload = *PROFILING_TRACE_RELOAD.read();
    let trace_limit = *PROFILING_CHROME_LIMIT.read();
    let config = use_app_resource(|| async move {
        let result = ApiClient::new().get_profiler_config().await;
        match &result {
            Ok(config) => apply_profiler_config(config),
            Err(_) => *PROFILING_CONFIG_LOADED.write() = true,
        }
        result
    });
    config.suspend()?;

    match view.as_str() {
        "pprof" | "torch" => rsx! { FlamegraphLoader { key: "{view}", view } },
        "trace" => rsx! { TraceChromeTimelineLoader {
            key: "{trace_reload}-{trace_limit}",
            reload_key: trace_reload,
            limit: trace_limit,
        }},
        "pytorch" => rsx! { PytorchTimeline {} },
        "ray" => rsx! { RayTimeline {} },
        _ => rsx! { div {} },
    }
}

#[component]
fn FlamegraphLoader(view: String) -> Element {
    let active = match view.as_str() {
        "pprof" => *PROFILING_PPROF_FREQ.read() > 0,
        "torch" => *PROFILING_TORCH_ENABLED.read(),
        _ => false,
    };
    let profiler = if view == "pprof" { "pprof" } else { "torch" };
    if !active {
        return rsx! { ProfilerDisabledNotice { profiler_name: profiler } };
    }
    rsx! {
        AsyncBoundary {
            message: Some("Loading flamegraph".to_string()),
            FlamegraphData { key: "{profiler}", profiler: profiler.to_string() }
        }
    }
}

#[component]
fn FlamegraphData(profiler: String) -> Element {
    let is_torch = profiler == "torch";
    let is_pprof = profiler == "pprof";
    let mut metric = use_signal(|| "duration".to_string());
    let request_profiler = profiler.clone();
    let thread_tid = is_pprof.then(|| *PROFILING_THREAD_FILTER.read()).flatten();
    let thread_label = INVESTIGATION_CONTEXT.read().label.clone();
    let thread_display =
        thread_tid.map(|tid| thread_label.clone().unwrap_or_else(|| format!("tid {tid}")));
    let payload = use_app_resource(move || {
        let profiler = request_profiler.clone();
        let selected_metric = metric();
        async move {
            let body = if profiler == "torch" {
                ApiClient::new()
                    .get_flamegraph_json_with_metric(&profiler, Some(&selected_metric))
                    .await?
            } else {
                ApiClient::new().get_flamegraph_json(&profiler).await?
            };
            serde_json::from_str::<FlamegraphPayload>(&body).map_err(|error| {
                crate::utils::error::AppError::Api(format!("Invalid flamegraph JSON: {error}"))
            })
        }
    });

    match payload.suspend()?() {
        Ok(data) => rsx! {
            div { class: "flex h-full min-h-0 flex-col",
                if let Some(label) = thread_display {
                    div { class: "flex items-center gap-2 border-b border-blue-100 bg-blue-50 px-4 py-2 text-xs text-blue-900",
                        span { "Thread: {label}" }
                        button { class: "font-medium text-blue-700 hover:underline", onclick: move |_| clear_profiling_thread_filter(), "Clear" }
                    }
                }
                ProfileSnapshotBar {
                    key: "{profiler}-{metric()}",
                    profiler: profiler.clone(),
                    metric: if is_torch { Some(metric()) } else { None },
                    payload: data.clone(),
                }
                FlamegraphView {
                    key: "{profiler}-{metric()}-{thread_tid:?}",
                    payload: data,
                    thread_tid,
                    torch_metric: is_torch.then_some(metric),
                    on_torch_metric: is_torch.then(|| EventHandler::new(move |value| metric.set(value))),
                }
            }
        },
        Err(error) => {
            rsx! { ProfilingErrorPanel { title: "Flamegraph Error".to_string(), error: error.display_message() } }
        }
    }
}

#[component]
fn PytorchTimeline() -> Element {
    let reload = *PROFILING_PYTORCH_TIMELINE_RELOAD.read();
    if reload == 0 {
        rsx! { TimelinePlaceholder { title: "PyTorch Profiler Timeline", hint: "Use the capture controls, then load the timeline.".to_string() } }
    } else {
        rsx! { PytorchChromeTimelineLoader { reload_key: reload } }
    }
}

#[component]
fn RayTimeline() -> Element {
    let reload = *PROFILING_RAY_TIMELINE_RELOAD.read();
    if reload == 0 {
        rsx! { TimelinePlaceholder { title: "Ray Timeline", hint: "Use Reload Ray Timeline in the capture controls.".to_string() } }
    } else {
        rsx! { RayChromeTimelineLoader { reload_key: reload } }
    }
}
