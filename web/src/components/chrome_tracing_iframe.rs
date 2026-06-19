use dioxus::prelude::*;

use crate::components::common::ErrorState;
use crate::utils::tracing_viewer;

/// Renders a Chrome Tracing iframe from already-loaded JSON.
#[component]
pub fn ChromeTracingContent(
    trace_json: String,
    iframe_key: i32,
    #[props(optional)] empty_message: Option<String>,
    #[props(optional)] empty_title: Option<String>,
) -> Element {
    let empty_msg = empty_message.as_deref().unwrap_or("Timeline data is empty.");
    let empty_ttl = empty_title.as_deref().unwrap_or("Empty Timeline Data");

    if trace_json.trim().is_empty() {
        return rsx! {
            ErrorState {
                error: empty_msg.to_string(),
                title: Some(empty_ttl.to_string()),
            }
        };
    }
    if let Err(e) = serde_json::from_str::<serde_json::Value>(&trace_json) {
        return rsx! {
            ErrorState {
                error: format!("Invalid JSON: {:?}", e),
                title: Some("Invalid Timeline Data".to_string()),
            }
        };
    }

    rsx! {
        div {
            class: "absolute inset-0 overflow-hidden",
            style: "min-height: 600px;",
            iframe {
                key: "{iframe_key}",
                srcdoc: tracing_viewer::get_tracing_viewer_html(&trace_json),
                style: "width: 100%; height: 100%; border: none;",
                title: "Chrome Tracing Viewer"
            }
        }
    }
}
