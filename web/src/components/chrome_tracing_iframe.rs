use dioxus::prelude::*;

use crate::components::common::{ErrorState, LoadingState};
use crate::hooks::ApiState;
use crate::utils::tracing_viewer;

/// Shared Chrome Tracing iframe viewer. Renders loading/error/empty or iframe from trace JSON state.
#[component]
pub fn ChromeTracingIframe(
    state: ApiState<String>,
    iframe_key: Signal<i32>,
    #[props(optional)] loading_message: Option<String>,
    #[props(optional)] empty_message: Option<String>,
    #[props(optional)] empty_title: Option<String>,
    #[props(optional)] error_title: Option<String>,
) -> Element {
    let loading_msg = loading_message.as_deref().unwrap_or("Loading timeline...");
    let empty_msg = empty_message.as_deref().unwrap_or("Timeline data is empty.");
    let empty_ttl = empty_title.as_deref().unwrap_or("Empty Timeline Data");
    let err_ttl = error_title.as_deref().unwrap_or("Load Timeline Error");

    if state.is_loading() {
        return rsx! {
            LoadingState { message: Some(loading_msg.to_string()) }
        };
    }

    if let Some(Ok(ref trace_json)) = state.data.read().as_ref() {
        if trace_json.trim().is_empty() {
            return rsx! {
                ErrorState {
                    error: empty_msg.to_string(),
                    title: Some(empty_ttl.to_string())
                }
            };
        }
        if let Err(e) = serde_json::from_str::<serde_json::Value>(trace_json) {
            return rsx! {
                ErrorState {
                    error: format!("Invalid JSON: {:?}", e),
                    title: Some("Invalid Timeline Data".to_string())
                }
            };
        }
        return rsx! {
            div {
                class: "absolute inset-0 overflow-hidden",
                style: "min-height: 600px;",
                iframe {
                    key: "{*iframe_key.read()}",
                    srcdoc: tracing_viewer::get_tracing_viewer_html(trace_json),
                    style: "width: 100%; height: 100%; border: none;",
                    title: "Chrome Tracing Viewer"
                }
            }
        };
    }

    if let Some(Err(ref err)) = state.data.read().as_ref() {
        return rsx! {
            ErrorState {
                error: format!("Failed to load timeline: {:?}", err),
                title: Some(err_ttl.to_string())
            }
        };
    }

    rsx! {
        div {
            class: "absolute inset-0 flex items-center justify-center p-8",
            div { class: "text-center text-gray-500", "No data" }
        }
    }
}
