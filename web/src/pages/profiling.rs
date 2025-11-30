use dioxus::prelude::*;
use crate::components::common::{LoadingState, ErrorState, EmptyState};
use crate::components::page::{PageContainer, PageTitle};
use crate::hooks::use_api_simple;
use crate::api::{ApiClient, ProfileResponse};
use crate::app::{PROFILING_VIEW, PROFILING_PPROF_FREQ, PROFILING_TORCH_ENABLED,
    PROFILING_CHROME_LIMIT, PROFILING_PYTORCH_TIMELINE_RELOAD, PROFILING_RAY_TIMELINE_RELOAD};
use crate::pages::chrome_tracing::get_tracing_viewer_html;

fn apply_config(config: &[(String, String)]) {
    *PROFILING_PPROF_FREQ.write() = 0;
    *PROFILING_TORCH_ENABLED.write() = false;

    for (name, value) in config {
        match name.as_str() {
            "probing.pprof.sample_freq" => {
                if let Ok(v) = value.parse::<i32>() {
                    *PROFILING_PPROF_FREQ.write() = v.max(0);
                }
            },
            "probing.torch.profiling" => {
                let lowered = value.trim().to_lowercase();
                let disabled_values = ["", "0", "false", "off", "disable", "disabled"];
                let enabled = !disabled_values.contains(&lowered.as_str());
                *PROFILING_TORCH_ENABLED.write() = enabled;
            },
            _ => {}
        }
    }
}

#[component]
pub fn Profiling() -> Element {
    let chrome_iframe_key = use_signal(|| 0);

    let config_state = use_api_simple::<Vec<(String, String)>>();
    let flamegraph_state = use_api_simple::<String>();
    let chrome_tracing_state = use_api_simple::<String>();
    let pytorch_profile_state = use_api_simple::<ProfileResponse>();
    let ray_timeline_state = use_api_simple::<String>(); // Changed to String for Chrome format JSON

    use_effect(move || {
        let mut loading = config_state.loading;
        let mut data = config_state.data;
        spawn(async move {
            *loading.write() = true;
            let client = ApiClient::new();
            let result = client.get_profiler_config().await;
            match result {
                Ok(ref config) => {
                    apply_config(config);
                }
                Err(_) => {}
            }
            *data.write() = Some(result);
            *loading.write() = false;
        });
    });

    use_effect(move || {
        let view = PROFILING_VIEW.read().clone();
        drop(view);
        spawn(async move {
            let client = ApiClient::new();
            if let Ok(config) = client.get_profiler_config().await {
                apply_config(&config);
            }
        });
    });

    use_effect(move || {
        let view = PROFILING_VIEW.read().clone();
        let pprof_on = *PROFILING_PPROF_FREQ.read() > 0;
        let torch = *PROFILING_TORCH_ENABLED.read();

        let active_profiler = match view.as_str() {
            "pprof" if pprof_on => "pprof",
            "torch" if torch => "torch",
            _ => return,
        };

        let mut loading = flamegraph_state.loading;
        let mut data = flamegraph_state.data;
        spawn(async move {
            *loading.write() = true;
            let client = ApiClient::new();
            let result = client.get_flamegraph(active_profiler).await;
            *data.write() = Some(result);
            *loading.write() = false;
        });
    });

    // Handle automatic loading of trace timeline
    use_effect(move || {
        let view = PROFILING_VIEW.read().clone();
        let limit_val = *PROFILING_CHROME_LIMIT.read();

        if view != "trace-timeline" {
            return;
        }

        let mut loading = chrome_tracing_state.loading;
        let mut data = chrome_tracing_state.data;
        let mut iframe_key = chrome_iframe_key.clone();
        spawn(async move {
            *loading.write() = true;
            let client = ApiClient::new();
            let result = client.get_chrome_tracing_json(Some(limit_val)).await;
            *data.write() = Some(result);
            *loading.write() = false;
            *iframe_key.write() += 1;
        });
    });

    // Handle pytorch timeline loading (triggered by sidebar button)
    use_effect(move || {
        let view = PROFILING_VIEW.read().clone();
        let reload_key = *PROFILING_PYTORCH_TIMELINE_RELOAD.read();

        if view != "pytorch-timeline" || reload_key == 0 {
            return;
        }

        let mut loading = chrome_tracing_state.loading;
        let mut data = chrome_tracing_state.data;
        let mut iframe_key = chrome_iframe_key.clone();
        spawn(async move {
            *loading.write() = true;
            let client = ApiClient::new();
            let result = client.get_pytorch_timeline().await;
            *data.write() = Some(result);
            *loading.write() = false;
            *iframe_key.write() += 1;
        });
    });

    // Handle Ray timeline loading (Chrome format for Perfetto)
    use_effect(move || {
        let view = PROFILING_VIEW.read().clone();
        let reload_key = *PROFILING_RAY_TIMELINE_RELOAD.read();

        if view != "ray-timeline" || reload_key == 0 {
            return;
        }

        let mut loading = ray_timeline_state.loading;
        let mut data = ray_timeline_state.data;
        let mut iframe_key = chrome_iframe_key.clone();
        spawn(async move {
            *loading.write() = true;
            let client = ApiClient::new();
            let result = client.get_ray_timeline_chrome_format(None, None, None, None).await;
            *data.write() = Some(result);
            *loading.write() = false;
            *iframe_key.write() += 1; // Force iframe reload
        });
    });

    rsx! {
        PageContainer {
            {
                let current_view = PROFILING_VIEW.read();
                let (title, subtitle, icon) = match current_view.as_str() {
                    "pprof" => (
                        "pprof Flamegraph".to_string(),
                        Some("CPU profiling with pprof".to_string()),
                        Some(&icondata::CgPerformance),
                    ),
                    "torch" => (
                        "torch Flamegraph".to_string(),
                        Some("PyTorch profiling visualization".to_string()),
                        Some(&icondata::SiPytorch),
                    ),
                    "trace-timeline" => (
                        "Trace Timeline".to_string(),
                        Some("Chrome Tracing timeline view".to_string()),
                        Some(&icondata::AiThunderboltOutlined),
                    ),
                    "pytorch-timeline" => (
                        "PyTorch Timeline".to_string(),
                        Some("PyTorch profiler timeline view".to_string()),
                        Some(&icondata::SiPytorch),
                    ),
                    "ray-timeline" => (
                        "Ray Timeline".to_string(),
                        Some("Ray task and actor execution timeline".to_string()),
                        Some(&icondata::AiClockCircleOutlined),
                    ),
                    _ => (
                        "Profiling".to_string(),
                        Some("Performance profiling and analysis".to_string()),
                        Some(&icondata::AiSearchOutlined),
                    ),
                };
                rsx! {
                    PageTitle {
                        title,
                        subtitle,
                        icon,
                    }
                }
            }

            div {
                class: "bg-white rounded-lg shadow-sm border border-gray-200 relative",
                style: "min-height: calc(100vh - 12rem);",
                {
                    let current_view = PROFILING_VIEW.read().clone();
                    if current_view == "pprof" || current_view == "torch" {
                        rsx! {
                            FlamegraphView {
                                flamegraph_state: flamegraph_state.clone(),
                            }
                        }
                    } else if current_view == "trace-timeline" || current_view == "pytorch-timeline" {
                        rsx! {
                            ChromeTracingView {
                                chrome_tracing_state: chrome_tracing_state.clone(),
                                chrome_iframe_key: chrome_iframe_key.clone(),
                            }
                        }
                    } else if current_view == "ray-timeline" {
                        rsx! {
                            RayTimelineView {
                                ray_timeline_state: ray_timeline_state.clone(),
                                chrome_iframe_key: chrome_iframe_key.clone(),
                            }
                        }
                    } else {
                        rsx! { div {} }
                    }
                }
            }

        }
    }
}

#[component]
fn FlamegraphView(
    #[props] flamegraph_state: crate::hooks::ApiState<String>
) -> Element {
    let pprof_enabled = *PROFILING_PPROF_FREQ.read() > 0;
    let torch_enabled = *PROFILING_TORCH_ENABLED.read();
    let current_view = PROFILING_VIEW.read().clone();
    let profiler_name = if current_view == "pprof" { "pprof" } else { "torch" };

    if !pprof_enabled && !torch_enabled {
        let message = format!(
            "No profilers are currently enabled. Enable {} using the controls in the sidebar.",
            profiler_name
        );
        return rsx! {
            div {
                class: "absolute inset-0 flex items-center justify-center",
                div {
                    class: "text-center",
                    h2 { class: "text-2xl font-bold text-gray-900 mb-4", "No Profilers Enabled" }
                    EmptyState { message }
                }
            }
        };
    }

    if flamegraph_state.is_loading() {
        return rsx! {
            LoadingState { message: Some("Loading flamegraph...".to_string()) }
        };
    }

    if let Some(Ok(flamegraph)) = flamegraph_state.data.read().as_ref() {
        return rsx! {
            div {
                class: "absolute inset-0 w-full h-full",
                div {
                    class: "w-full h-full",
                    dangerous_inner_html: "{flamegraph}"
                }
            }
        };
    }

    if let Some(Err(err)) = flamegraph_state.data.read().as_ref() {
        return rsx! {
            ErrorState {
                error: format!("Failed to load flamegraph: {:?}", err),
                title: Some("Error Loading Flamegraph".to_string())
            }
        };
    }

    rsx! { div {} }
}

#[component]
fn ChromeTracingView(
    #[props] chrome_tracing_state: crate::hooks::ApiState<String>,
    #[props] chrome_iframe_key: Signal<i32>,
) -> Element {
    let current_view = PROFILING_VIEW.read().clone();
    let is_pytorch = current_view == "pytorch-timeline";

    if chrome_tracing_state.is_loading() {
        let message = if is_pytorch {
            "Loading PyTorch timeline data..."
        } else {
            "Loading trace data..."
        };
        return rsx! {
            LoadingState { message: Some(message.to_string()) }
        };
    }

    if let Some(Ok(ref trace_json)) = chrome_tracing_state.data.read().as_ref() {
        if trace_json.trim().is_empty() {
            return rsx! {
                ErrorState {
                    error: "Timeline data is empty. Make sure the profiler has been executed.".to_string(),
                    title: Some("Empty Timeline Data".to_string())
                }
            };
        }

        if let Err(e) = serde_json::from_str::<serde_json::Value>(trace_json) {
            return rsx! {
                ErrorState {
                    error: format!("Invalid JSON data: {:?}", e),
                    title: Some("Invalid Timeline Data".to_string())
                }
            };
        }

        return rsx! {
            div {
                class: "absolute inset-0 overflow-hidden",
                style: "min-height: 600px;",
                iframe {
                    key: "{*chrome_iframe_key.read()}",
                    srcdoc: get_tracing_viewer_html(trace_json),
                    style: "width: 100%; height: 100%; border: none;",
                    title: "Chrome Tracing Viewer"
                }
            }
        };
    }

    if let Some(Err(ref err)) = chrome_tracing_state.data.read().as_ref() {
        return rsx! {
            ErrorState {
                error: format!("Failed to load timeline: {:?}", err),
                title: Some("Load Timeline Error".to_string())
            }
        };
    }

    let (title, description) = if is_pytorch {
        ("PyTorch Profiler Timeline",
         "Click 'Start Profile' to begin profiling, then click 'Load Timeline' to view the results.")
    } else {
        ("Trace Events Timeline",
         "Select the number of events and the timeline will load automatically.")
    };

    rsx! {
        div {
            class: "absolute inset-0 flex items-center justify-center p-8",
            div {
                class: "text-center text-gray-500",
                p {
                    class: "mb-4 text-lg",
                    "{title}"
                }
                p {
                    class: "text-sm",
                    "{description}"
                }
            }
        }
    }
}

#[component]
fn RayTimelineView(
    #[props] ray_timeline_state: crate::hooks::ApiState<String>,
    #[props] chrome_iframe_key: Signal<i32>,
) -> Element {
    if ray_timeline_state.is_loading() {
        return rsx! {
            LoadingState { message: Some("Loading Ray timeline data...".to_string()) }
        };
    }

    if let Some(Ok(ref trace_json)) = ray_timeline_state.data.read().as_ref() {
        if trace_json.trim().is_empty() {
            return rsx! {
                ErrorState {
                    error: "No Ray timeline data available. Start Ray tasks with probing tracing enabled.".to_string(),
                    title: Some("Empty Timeline Data".to_string())
                }
            };
        }

        // Validate JSON
        if let Err(e) = serde_json::from_str::<serde_json::Value>(trace_json) {
            return rsx! {
                ErrorState {
                    error: format!("Invalid JSON data: {:?}", e),
                    title: Some("Invalid Timeline Data".to_string())
                }
            };
        }

        // Use Perfetto UI to display (same as Chrome tracing)
        return rsx! {
            div {
                class: "bg-white rounded-lg shadow overflow-hidden",
                style: "height: calc(100vh - 300px); min-height: 600px;",
                iframe {
                    key: "{*chrome_iframe_key.read()}",
                    srcdoc: get_tracing_viewer_html(trace_json),
                    style: "width: 100%; height: 100%; border: none;",
                    title: "Ray Timeline Viewer (Perfetto)"
                }
            }
        };
    }

    if let Some(Err(err)) = ray_timeline_state.data.read().as_ref() {
        return rsx! {
            ErrorState {
                error: format!("Failed to load Ray timeline: {:?}", err),
                title: Some("Load Timeline Error".to_string())
            }
        };
    }

    rsx! {
        div {
            class: "bg-white rounded-lg shadow p-8 text-center",
            div {
                class: "text-gray-500",
                p {
                    class: "mb-4 text-lg",
                    "Ray Timeline"
                }
                p {
                    class: "text-sm",
                    "Click 'Reload Ray Timeline' to load the timeline data."
                }
            }
        }
    }
}


#[component]
fn PyTorchProfileStatus(profile_result: ProfileResponse) -> Element {
    if profile_result.success {
        let message = profile_result.message.as_deref().unwrap_or("Profile started successfully");
        rsx! {
            div {
                class: "p-3 bg-green-50 border border-green-200 rounded text-sm text-green-800",
                "{message}"
            }
        }
    } else {
        let error = profile_result.error.as_deref().unwrap_or("Failed to start profile");
        rsx! {
            div {
                class: "p-3 bg-red-50 border border-red-200 rounded text-sm text-red-800",
                "{error}"
            }
        }
    }
}
