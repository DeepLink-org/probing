use dioxus::prelude::*;

use crate::api::ApiClient;
use crate::components::common::AsyncBoundary;
use crate::components::flamegraph::{FlamegraphPayload, FlamegraphView};
use crate::components::profiling_sidebar_hint::ProfilingSidebarHint;
use crate::components::page::PageTitle;
use crate::components::profile_snapshot_bar::ProfileSnapshotBar;
use crate::components::profiling::{
    ProfilerDisabledNotice, ProfilingContentPanel, ProfilingErrorPanel, ProfilingFeedbackToast,
    PytorchChromeTimelineLoader, RayChromeTimelineLoader, TimelinePlaceholder,
    TraceChromeTimelineLoader,
};
use crate::hooks::use_app_resource;
use crate::state::profiling::{
    apply_profiler_config, normalize_profiling_view, profiling_view_spec, PROFILING_CHROME_LIMIT,
    PROFILING_CONFIG_LOADED, PROFILING_PPROF_FREQ, PROFILING_PYTORCH_TIMELINE_RELOAD,
    PROFILING_RAY_TIMELINE_RELOAD, PROFILING_TORCH_ENABLED, PROFILING_TRACE_RELOAD,
};

#[component]
pub fn Profiling(view: String) -> Element {
    let current_view = normalize_profiling_view(&view).to_string();
    let spec = profiling_view_spec(&current_view);
    let title = spec.label.to_string();
    let subtitle = view_subtitle(&current_view);

    rsx! {
        ProfilingFeedbackToast {}
        div {
            class: "flex flex-col flex-1 min-h-0 h-full gap-4",
            PageTitle {
                title,
                subtitle: Some(subtitle),
                icon: Some(view_icon(&current_view)),
            }
            ProfilingSidebarHint {}
            div { class: "flex flex-col flex-1 min-h-0 min-w-0",
                ProfilingContentPanel {
                    AsyncBoundary {
                        message: Some("Loading profiler configuration…".to_string()),
                        ProfilerConfigGate { key: "{current_view}", view: current_view }
                    }
                }
            }
        }
    }
}

fn view_icon(view: &str) -> &'static icondata::Icon {
    match view {
        "pprof" => &icondata::CgPerformance,
        "torch" => &icondata::SiPytorch,
        "trace" => &icondata::AiThunderboltOutlined,
        "pytorch" => &icondata::SiPytorch,
        "ray" => &icondata::AiClockCircleOutlined,
        _ => &icondata::AiSearchOutlined,
    }
}

fn view_subtitle(view: &str) -> String {
    match view {
        "pprof" => "SIGPROF stack explorer · statistical sampling".to_string(),
        "torch" => "Median post-hook duration · statistical sampling".to_string(),
        "trace" => "Chrome trace events from probing buffers — not distributed spans".to_string(),
        "pytorch" => "PyTorch profiler chrome trace".to_string(),
        "ray" => "Ray task timeline".to_string(),
        _ => "Profiling views".to_string(),
    }
}

#[component]
fn ProfilerConfigGate(view: String) -> Element {
    let trace_reload = *PROFILING_TRACE_RELOAD.read();
    let trace_limit = *PROFILING_CHROME_LIMIT.read();

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

    match view.as_str() {
        "pprof" | "torch" => rsx! {
            AsyncBoundary {
                message: Some("Loading flamegraph…".to_string()),
                FlamegraphLoader { key: "{view}", view: view.clone() }
            }
        },
        "trace" => rsx! {
            AsyncBoundary {
                message: Some("Loading trace data…".to_string()),
                TraceChromeTimelineLoader {
                    key: "{view}-{trace_reload}-{trace_limit}",
                    reload_key: trace_reload,
                    limit: trace_limit,
                }
            }
        },
        "pytorch" => rsx! {
            AsyncBoundary {
                message: Some("Loading PyTorch timeline data…".to_string()),
                PytorchTimelineLoader { key: "{view}" }
            }
        },
        "ray" => rsx! {
            AsyncBoundary {
                message: Some("Loading Ray timeline data…".to_string()),
                RayTimelineLoader { key: "{view}" }
            }
        },
        _ => rsx! { div {} },
    }
}

#[component]
fn FlamegraphLoader(view: String) -> Element {
    let pprof_enabled = *PROFILING_PPROF_FREQ.read() > 0;
    let torch_enabled = *PROFILING_TORCH_ENABLED.read();
    let profiler_name = if view == "pprof" { "pprof" } else { "torch" };

    let profiler_active = match view.as_str() {
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
            div { class: "flex flex-col flex-1 min-h-[600px]",
                ProfileSnapshotBar {
                    key: "{profiler_name}-{metric()}",
                    profiler: profiler_name.clone(),
                    metric: if is_torch { Some(metric()) } else { None },
                    payload: data.clone(),
                }
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
                hint: "Use Start Profile and Load Timeline in the sidebar.".to_string(),
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
                hint: "Click Reload Ray Timeline in the sidebar.".to_string(),
            }
        };
    }

    rsx! {
        RayChromeTimelineLoader { reload_key }
    }
}
