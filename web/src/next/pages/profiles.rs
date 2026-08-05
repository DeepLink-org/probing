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
    PROFILING_CONFIG_LOADED, PROFILING_PPROF_FREQ, PROFILING_PYTORCH_STEPS,
    PROFILING_PYTORCH_TIMELINE_RELOAD, PROFILING_RAY_TIMELINE_RELOAD, PROFILING_TORCH_ENABLED,
    PROFILING_TRACE_RELOAD,
};

use super::super::components::{EvidenceMetric, WorkspacePage};

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
            title: "Profiling".to_string(),
            subtitle: format!("{} · {}", spec.label, view_subtitle(&current)),
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
            Err(_) => *PROFILING_CONFIG_LOADED.write() = false,
        }
        result
    });
    let config_result = config.suspend()?();
    let config_error = profiler_config_failure(&config_result);
    if profile_requires_runtime_config(&view) && config_error.is_some() {
        return rsx! { ProfilingErrorPanel {
            title: "Profiler configuration unavailable".to_string(),
            error: config_error.unwrap_or_default(),
        }};
    }

    rsx! {
        div { class: "flex h-full min-h-0 flex-col",
            ProfilingEvidenceBar { view: view.clone(), config_loaded: config_error.is_none() }
            if let Some(error) = config_error {
                div { class: "border-b border-amber-200 bg-amber-50 px-4 py-2 text-xs text-amber-900",
                    "Profiler settings unavailable: {error}. This capture view has independent availability."
                }
            }
            div { class: "min-h-0 flex-1",
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
        }
    }
}

fn profile_requires_runtime_config(view: &str) -> bool {
    matches!(view, "pprof" | "torch")
}

#[component]
fn ProfilingEvidenceBar(view: String, config_loaded: bool) -> Element {
    let (state, sample_window) = match view.as_str() {
        "pprof" => {
            let frequency = *PROFILING_PPROF_FREQ.read();
            (
                if frequency > 0 {
                    format!("Enabled · {frequency} Hz")
                } else {
                    "Disabled".to_string()
                },
                "Accumulated statistical samples".to_string(),
            )
        }
        "torch" => (
            if *PROFILING_TORCH_ENABLED.read() {
                "Enabled".to_string()
            } else {
                "Disabled".to_string()
            },
            "Reported module-hook samples".to_string(),
        ),
        "trace" => (
            if *PROFILING_TRACE_RELOAD.read() > 0 {
                "Reload requested".to_string()
            } else {
                "No reload requested".to_string()
            },
            format!("Up to {} buffer events", *PROFILING_CHROME_LIMIT.read()),
        ),
        "pytorch" => (
            if *PROFILING_PYTORCH_TIMELINE_RELOAD.read() > 0 {
                "Load requested".to_string()
            } else {
                "No load requested".to_string()
            },
            format!(
                "{} steps per requested capture",
                *PROFILING_PYTORCH_STEPS.read()
            ),
        ),
        "ray" => (
            if *PROFILING_RAY_TIMELINE_RELOAD.read() > 0 {
                "Reload requested".to_string()
            } else {
                "No reload requested".to_string()
            },
            "Latest requested Ray timeline".to_string(),
        ),
        _ => ("Unknown view".to_string(), "—".to_string()),
    };
    let thread = if view == "pprof" {
        (*PROFILING_THREAD_FILTER.read())
            .map(|tid| format!("Thread {tid}"))
            .unwrap_or_else(|| "All sampled threads".to_string())
    } else {
        "Current process".to_string()
    };
    let settings = if profile_requires_runtime_config(&view) {
        if config_loaded {
            "Loaded"
        } else {
            "Unavailable"
        }
    } else {
        "Independent"
    };
    rsx! {
        div { class: "grid shrink-0 grid-cols-4 divide-x divide-gray-200 border-b border-gray-200 bg-gray-50/80 px-4 py-3",
            EvidenceMetric { label: "Scope", value: "Current process".to_string(), detail: Some(thread) }
            EvidenceMetric { label: "Capture state", value: state }
            EvidenceMetric { label: "Sample window", value: sample_window }
            EvidenceMetric { label: "Profiler settings", value: settings.to_string() }
        }
    }
}

fn profiler_config_failure(
    result: &crate::utils::error::Result<Vec<(String, String)>>,
) -> Option<String> {
    result.as_ref().err().map(|error| error.display_message())
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
        rsx! { TimelinePlaceholder { title: "PyTorch profiler timeline", hint: "No PyTorch profiler timeline has been loaded in this browser session.".to_string() } }
    } else {
        rsx! { PytorchChromeTimelineLoader { reload_key: reload } }
    }
}

#[component]
fn RayTimeline() -> Element {
    let reload = *PROFILING_RAY_TIMELINE_RELOAD.read();
    if reload == 0 {
        rsx! { TimelinePlaceholder { title: "Ray timeline", hint: "No Ray timeline has been loaded in this browser session.".to_string() } }
    } else {
        rsx! { RayChromeTimelineLoader { reload_key: reload } }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::error::AppError;

    #[test]
    fn profiler_config_failure_is_not_treated_as_loaded_configuration() {
        let failure = Err(AppError::Api("settings query failed".into()));
        assert_eq!(
            profiler_config_failure(&failure).as_deref(),
            Some("settings query failed")
        );

        let success = Ok(vec![("probing.pprof.frequency".into(), "99".into())]);
        assert_eq!(profiler_config_failure(&success), None);
    }

    #[test]
    fn only_config_backed_flamegraphs_are_blocked_by_config_failure() {
        assert!(profile_requires_runtime_config("pprof"));
        assert!(profile_requires_runtime_config("torch"));
        assert!(!profile_requires_runtime_config("trace"));
        assert!(!profile_requires_runtime_config("pytorch"));
        assert!(!profile_requires_runtime_config("ray"));
    }
}
