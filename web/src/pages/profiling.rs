use dioxus::prelude::*;

use crate::api::ApiClient;
use crate::components::common::AsyncBoundary;
use crate::components::flamegraph::{FlamegraphPayload, FlamegraphView};
use crate::components::page::{PageContainer, PageTitle};
use crate::components::profiling::{
    ProfilerDisabledNotice, ProfilingContentPanel, ProfilingErrorPanel, PytorchChromeTimelineLoader,
    RayChromeTimelineLoader, TimelinePlaceholder, TraceChromeTimelineLoader,
};
use crate::hooks::use_app_resource;
use crate::state::profiling::{
    apply_profiler_config, PROFILING_CHROME_LIMIT, PROFILING_CONFIG_LOADED, PROFILING_PPROF_FREQ,
    PROFILING_PYTORCH_TIMELINE_RELOAD, PROFILING_RAY_TIMELINE_RELOAD, PROFILING_TORCH_ENABLED,
    PROFILING_VIEW,
};

#[component]
pub fn Profiling() -> Element {
    let current_view = PROFILING_VIEW.read().clone();
    let (title, subtitle, icon) = view_title(&current_view);

    rsx! {
        PageContainer {
            PageTitle {
                title,
                subtitle: Some(subtitle),
                icon: Some(icon),
            }
            ProfilingContentPanel {
                AsyncBoundary {
                    message: Some("Loading profiler configuration…".to_string()),
                    ProfilerConfigGate {}
                }
            }
        }
    }
}

fn view_title(view: &str) -> (String, String, &'static icondata::Icon) {
    match view {
        "pprof" => (
            "CPU sampling".to_string(),
            "SIGPROF stack explorer · statistical sampling".to_string(),
            &icondata::CgPerformance,
        ),
        "torch" => (
            "Module performance".to_string(),
            "Median post-hook duration · statistical sampling".to_string(),
            &icondata::SiPytorch,
        ),
        "trace-timeline" => (
            "Trace".to_string(),
            "Chrome timeline".to_string(),
            &icondata::AiThunderboltOutlined,
        ),
        "pytorch-timeline" => (
            "PyTorch".to_string(),
            "Profiler timeline".to_string(),
            &icondata::SiPytorch,
        ),
        "ray-timeline" => (
            "Ray".to_string(),
            "Ray timeline".to_string(),
            &icondata::AiClockCircleOutlined,
        ),
        _ => (
            "Profiling".to_string(),
            "Profiling views".to_string(),
            &icondata::AiSearchOutlined,
        ),
    }
}

#[component]
fn ProfilerConfigGate() -> Element {
    let current_view = PROFILING_VIEW.read().clone();
    let _config = use_app_resource(|| async move {
        let client = ApiClient::new();
        let result = client.get_profiler_config().await;
        match &result {
            Ok(config) => apply_profiler_config(config),
            Err(_) => *PROFILING_CONFIG_LOADED.write() = true,
        }
        result
    });
    _config.suspend()?;

    match current_view.as_str() {
        "pprof" | "torch" => rsx! {
            AsyncBoundary {
                message: Some("Loading flamegraph…".to_string()),
                FlamegraphLoader { key: "{current_view}" }
            }
        },
        "trace-timeline" => rsx! {
            AsyncBoundary {
                message: Some("Loading trace data…".to_string()),
                TraceChromeTimelineLoader { limit: *PROFILING_CHROME_LIMIT.read() }
            }
        },
        "pytorch-timeline" => rsx! {
            AsyncBoundary {
                message: Some("Loading PyTorch timeline data…".to_string()),
                PytorchTimelineLoader {}
            }
        },
        "ray-timeline" => rsx! {
            AsyncBoundary {
                message: Some("Loading Ray timeline data…".to_string()),
                RayTimelineLoader {}
            }
        },
        _ => rsx! { div {} },
    }
}

#[component]
fn FlamegraphLoader() -> Element {
    let current_view = PROFILING_VIEW.read().clone();
    let pprof_enabled = *PROFILING_PPROF_FREQ.read() > 0;
    let torch_enabled = *PROFILING_TORCH_ENABLED.read();
    let profiler_name = if current_view == "pprof" { "pprof" } else { "torch" };

    let profiler_active = match current_view.as_str() {
        "pprof" => pprof_enabled,
        "torch" => torch_enabled,
        _ => false,
    };

    if !profiler_active {
        return rsx! {
            ProfilerDisabledNotice { profiler_name }
        };
    }

    rsx! {
        FlamegraphData {
            key: "{profiler_name}",
            profiler_name: profiler_name.to_string(),
        }
    }
}

#[component]
fn FlamegraphData(profiler_name: String) -> Element {
    let is_torch = profiler_name == "torch";
    let mut metric = use_signal(|| "duration".to_string());
    let fetch_name = profiler_name.clone();

    let payload = use_app_resource(move || {
        let name = fetch_name.clone();
        let m = metric();
        async move {
            let client = ApiClient::new();
            let body = if name == "torch" {
                client
                    .get_flamegraph_json_with_metric(&name, Some(&m))
                    .await?
            } else {
                client.get_flamegraph_json(&name).await?
            };
            let parsed: FlamegraphPayload = serde_json::from_str(&body)
                .map_err(|e| crate::utils::error::AppError::Api(format!("Invalid flamegraph JSON: {e}")))?;
            Ok(parsed)
        }
    });

    match payload.suspend()?() {
        Ok(data) => rsx! {
            FlamegraphView {
                key: "{profiler_name}-{metric()}",
                payload: data,
                torch_metric: if is_torch { Some(metric) } else { None },
                on_torch_metric: if is_torch {
                    Some(EventHandler::new(move |m: String| metric.set(m)))
                } else {
                    None
                },
            }
        },
        Err(err) => rsx! {
            ProfilingErrorPanel {
                title: "Flamegraph Error".to_string(),
                error: err.display_message(),
            }
        },
    }
}

#[component]
fn PytorchTimelineLoader() -> Element {
    let reload_key = *PROFILING_PYTORCH_TIMELINE_RELOAD.read();
    if reload_key == 0 {
        return rsx! {
            TimelinePlaceholder {
                title: "PyTorch Profiler Timeline",
                hint: "Click 'Start Profile' to begin profiling, then click 'Load Timeline' to view the results.".to_string(),
            }
        };
    }

    rsx! {
        PytorchChromeTimelineLoader { reload_key }
    }
}

#[component]
fn RayTimelineLoader() -> Element {
    let reload_key = *PROFILING_RAY_TIMELINE_RELOAD.read();
    if reload_key == 0 {
        return rsx! {
            TimelinePlaceholder {
                title: "Ray Timeline",
                hint: "Click 'Reload Ray Timeline' to load the timeline data.".to_string(),
            }
        };
    }

    rsx! {
        RayChromeTimelineLoader { reload_key }
    }
}
