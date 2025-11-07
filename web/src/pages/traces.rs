use dioxus::prelude::*;
use crate::components::card::Card;
use crate::components::page::{PageContainer, PageHeader};
use crate::components::common::{LoadingState, ErrorState};
use crate::hooks::use_api;
use crate::api::{ApiClient, SpanInfo, EventInfo};

#[component]
pub fn Traces() -> Element {
    let state = use_api(|| {
        let client = ApiClient::new();
        async move { client.get_span_tree().await }
    });

    rsx! {
        PageContainer {
            PageHeader {
                title: "Traces".to_string(),
                subtitle: Some("Analyze span timing and nested relationships".to_string())
            }
            
            Card {
                title: "Span Tree",
                if state.is_loading() {
                    LoadingState { message: Some("Loading trace data...".to_string()) }
                } else if let Some(Ok(spans)) = state.data.read().as_ref() {
                    if spans.is_empty() {
                        div {
                            class: "text-center py-8 text-gray-500",
                            "No trace data available. Start tracing with probing.tracing.span()"
                        }
                    } else {
                        div {
                            class: "space-y-4",
                            for span in spans.iter() {
                                SpanView { span: span.clone(), depth: 0 }
                            }
                        }
                    }
                } else if let Some(Err(err)) = state.data.read().as_ref() {
                    ErrorState { error: format!("{:?}", err), title: None }
                }
            }
        }
    }
}

#[component]
fn SpanView(span: SpanInfo, depth: usize) -> Element {
    let indent = depth * 24;
    let duration = span.end_timestamp
        .map(|end| (end - span.start_timestamp) as f64 / 1_000_000_000.0)
        .unwrap_or(0.0);
    
    let mut expanded = use_signal(|| depth < 2); // Auto-expand first 2 levels
    
    rsx! {
        div {
            class: "border-l-2 border-gray-200 pl-4",
            style: format!("margin-left: {}px", indent),
            div {
                class: "flex items-center gap-2 py-2 hover:bg-gray-50 rounded px-2",
                button {
                    class: "text-gray-400 hover:text-gray-600",
                    onclick: move |_| {
                        let current = *expanded.read();
                        *expanded.write() = !current;
                    },
                    if *expanded.read() {
                        "▼"
                    } else {
                        "▶"
                    }
                }
                span {
                    class: "font-semibold text-gray-900",
                    "{span.name}"
                }
                if let Some(ref kind) = span.kind {
                    span {
                        class: "text-xs px-2 py-0.5 bg-blue-100 text-blue-800 rounded",
                        "{kind}"
                    }
                }
                span {
                    class: "text-sm text-gray-500",
                    "span_id: {span.span_id}"
                }
                if let Some(ref parent_id) = span.parent_id {
                    span {
                        class: "text-sm text-gray-400",
                        "parent: {parent_id}"
                    }
                }
                span {
                    class: "text-sm text-gray-500",
                    "thread: {span.thread_id}"
                }
                span {
                    class: "text-sm font-mono text-green-600",
                    "{duration:.3}s"
                }
            }
            
            if *expanded.read() {
                div {
                    class: "ml-6 space-y-2",
                    // Events
                    if !span.events.is_empty() {
                        div {
                            class: "text-xs text-gray-500 mb-1",
                            "Events ({span.events.len()}):"
                        }
                        for event in span.events.iter() {
                            EventView { event: event.clone() }
                        }
                    }
                    
                    // Children spans
                    if !span.children.is_empty() {
                        div {
                            class: "text-xs text-gray-500 mb-1",
                            "Child Spans ({span.children.len()}):"
                        }
                        for child in span.children.iter() {
                            SpanView { span: child.clone(), depth: depth + 1 }
                        }
                    }
                    
                    // Attributes
                    if let Some(ref attrs) = span.attributes {
                        if !attrs.is_empty() {
                            div {
                                class: "text-xs text-gray-500 mt-2",
                                "Attributes:"
                            }
                            div {
                                class: "text-xs font-mono bg-gray-50 p-2 rounded mt-1",
                                "{attrs}"
                            }
                        }
                    }
                    
                    // Code path
                    if let Some(ref code_path) = span.code_path {
                        if !code_path.is_empty() {
                            div {
                                class: "text-xs text-gray-500 mt-2",
                                "Code Path:"
                            }
                            div {
                                class: "text-xs font-mono bg-gray-50 p-2 rounded mt-1",
                                "{code_path}"
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn EventView(event: EventInfo) -> Element {
    rsx! {
        div {
            class: "flex items-center gap-2 py-1 text-sm",
            span {
                class: "text-gray-400",
                "•"
            }
            span {
                class: "text-gray-700",
                "{event.name}"
            }
            if let Some(ref attrs) = event.attributes {
                if !attrs.is_empty() {
                    span {
                        class: "text-xs text-gray-500 font-mono",
                        "{attrs}"
                    }
                }
            }
        }
    }
}

