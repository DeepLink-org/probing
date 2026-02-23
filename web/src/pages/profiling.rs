use dioxus::prelude::*;
use crate::components::common::{LoadingState, ErrorState, EmptyState};
use crate::components::page::{PageContainer, PageTitle};
use crate::hooks::use_api_simple;
use crate::api::{ApiClient, ProfileResponse};
use crate::state::profiling::{
    PROFILING_CHROME_LIMIT, PROFILING_PPROF_FREQ, PROFILING_PYTORCH_TIMELINE_RELOAD,
    PROFILING_RAY_TIMELINE_RELOAD, PROFILING_TORCH_ENABLED, PROFILING_VIEW,
};
use crate::components::chrome_tracing_iframe::ChromeTracingIframe;

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
    let _pytorch_profile_state = use_api_simple::<ProfileResponse>();
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
                        "pprof".to_string(),
                        Some("CPU flamegraph".to_string()),
                        Some(&icondata::CgPerformance),
                    ),
                    "torch" => (
                        "torch".to_string(),
                        Some("PyTorch flamegraph".to_string()),
                        Some(&icondata::SiPytorch),
                    ),
                    "trace-timeline" => (
                        "Trace".to_string(),
                        Some("Chrome timeline".to_string()),
                        Some(&icondata::AiThunderboltOutlined),
                    ),
                    "pytorch-timeline" => (
                        "PyTorch".to_string(),
                        Some("Profiler timeline".to_string()),
                        Some(&icondata::SiPytorch),
                    ),
                    "ray-timeline" => (
                        "Ray".to_string(),
                        Some("Ray timeline".to_string()),
                        Some(&icondata::AiClockCircleOutlined),
                    ),
                    _ => (
                        "Profiling".to_string(),
                        Some("Profiling views".to_string()),
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
                class: "bg-white rounded-lg border border-gray-200 relative",
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
                error: format!("Failed to load flamegraph: {}", err.display_message()),
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

    let loading_msg = if is_pytorch {
        "Loading PyTorch timeline data..."
    } else {
        "Loading trace data..."
    };
    let empty_msg = "Timeline data is empty. Make sure the profiler has been executed.";

    let no_data_placeholder = (!chrome_tracing_state.is_loading()
        && chrome_tracing_state.data.read().as_ref().is_none())
    .then(|| {
        let (title, description) = if is_pytorch {
            (
                "PyTorch Profiler Timeline",
                "Click 'Start Profile' to begin profiling, then click 'Load Timeline' to view the results.",
            )
        } else {
            (
                "Trace Events Timeline",
                "Select the number of events and the timeline will load automatically.",
            )
        };
        rsx! {
            div {
                class: "absolute inset-0 flex items-center justify-center p-8",
                div {
                    class: "text-center text-gray-500",
                    p { class: "mb-4 text-lg", "{title}" }
                    p { class: "text-sm", "{description}" }
                }
            }
        }
    });

    if let Some(placeholder) = no_data_placeholder {
        return placeholder;
    }

    rsx! {
        ChromeTracingIframe {
            state: chrome_tracing_state.clone(),
            iframe_key: chrome_iframe_key,
            loading_message: Some(loading_msg.to_string()),
            empty_message: Some(empty_msg.to_string()),
        }
    }
}

#[component]
fn RayTimelineView(
    #[props] ray_timeline_state: crate::hooks::ApiState<String>,
    #[props] chrome_iframe_key: Signal<i32>,
) -> Element {
    let no_data_placeholder =
        (!ray_timeline_state.is_loading() && ray_timeline_state.data.read().as_ref().is_none())
            .then(|| {
                rsx! {
                    div {
                        class: "bg-white rounded-lg shadow p-8 text-center",
                        div {
                            class: "text-gray-500",
                            p { class: "mb-4 text-lg", "Ray Timeline" }
                            p { class: "text-sm", "Click 'Reload Ray Timeline' to load the timeline data." }
                        }
                    }
                }
            });

    if let Some(placeholder) = no_data_placeholder {
        return placeholder;
    }

    rsx! {
        ChromeTracingIframe {
            state: ray_timeline_state.clone(),
            iframe_key: chrome_iframe_key,
            loading_message: Some("Loading Ray timeline data...".to_string()),
            empty_message: Some("No Ray timeline data available. Start Ray tasks with probing tracing enabled.".to_string()),
        }
    }
}
