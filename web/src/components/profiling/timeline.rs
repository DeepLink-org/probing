//! Chrome tracing timeline loaders for the Profiling page.

use dioxus::prelude::*;

use crate::api::ApiClient;
use crate::components::timeline_viewer::TimelineViewer;
use crate::hooks::use_app_resource;

use super::sections::{ProfilingErrorPanel, TimelinePanel};

#[component]
fn ChromeTimelineResource(
    timeline: Resource<Result<String, crate::utils::error::AppError>>,
    empty_message: String,
    error_title: String,
) -> Element {
    match timeline.suspend()?() {
        Ok(json) => rsx! {
            TimelinePanel {
                TimelineViewer {
                    trace_json: json,
                    empty_message: Some(empty_message),
                }
            }
        },
        Err(err) => rsx! {
            ProfilingErrorPanel {
                title: error_title,
                error: format!("Failed to load timeline: {}", err.display_message()),
            }
        },
    }
}

#[component]
pub fn TraceChromeTimelineLoader(reload_key: i32, limit: usize) -> Element {
    let timeline = use_app_resource(move || {
        let _ = reload_key;
        let lim = limit;
        async move { ApiClient::new().get_chrome_tracing_json(Some(lim)).await }
    });

    rsx! {
        ChromeTimelineResource {
            timeline,
            empty_message: "Timeline data is empty. Make sure the profiler has been executed.".to_string(),
            error_title: "Load Timeline Error".to_string(),
        }
    }
}

#[component]
pub fn PytorchChromeTimelineLoader(reload_key: i32) -> Element {
    let timeline = use_app_resource(move || {
        let _ = reload_key;
        async move { ApiClient::new().get_pytorch_timeline().await }
    });

    rsx! {
        ChromeTimelineResource {
            timeline,
            empty_message: "Timeline data is empty. Make sure the profiler has been executed.".to_string(),
            error_title: "Load Timeline Error".to_string(),
        }
    }
}

#[component]
pub fn RayChromeTimelineLoader(reload_key: i32) -> Element {
    let timeline = use_app_resource(move || {
        let _ = reload_key;
        async move {
            ApiClient::new()
                .get_ray_timeline_chrome_format(None, None, None, None)
                .await
        }
    });

    rsx! {
        ChromeTimelineResource {
            timeline,
            empty_message: "No Ray timeline data available. Start Ray tasks with probing tracing enabled.".to_string(),
            error_title: "Load Timeline Error".to_string(),
        }
    }
}
