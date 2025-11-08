use dioxus::prelude::*;
use crate::components::card::Card;
use crate::components::page::{PageContainer, PageHeader};
use crate::components::common::{LoadingState, ErrorState};
use crate::hooks::use_api_simple;
use crate::api::{ApiClient, SpanInfo, EventInfo};

#[component]
pub fn Traces() -> Element {
    let limit = use_signal(|| 400usize);
    let state = use_api_simple::<Vec<SpanInfo>>();
    
    // 创建依赖项，当limit改变时重新计算
    let limit_value = use_memo({
        let limit = limit.clone();
        move || *limit.read()
    });
    
    // 当limit改变时重新获取数据
    use_effect({
        let limit_value = limit_value.clone();
        let mut loading = state.loading;
        let mut data = state.data;
        move || {
            let limit_val = *limit_value.read();
            spawn(async move {
                *loading.write() = true;
                let client = ApiClient::new();
                let result = client.get_span_tree(Some(limit_val)).await;
                *data.write() = Some(result);
                *loading.write() = false;
            });
        }
    });

    rsx! {
        PageContainer {
            PageHeader {
                title: "Traces".to_string(),
                subtitle: Some("Analyze span timing and nested relationships".to_string())
            }
            
            // Limit control slider
            Card {
                title: "Data Limit",
                div {
                    class: "space-y-2",
                    div {
                        class: "flex items-center justify-between",
                        span {
                            class: "text-sm text-gray-600",
                            "Number of Events"
                        }
                        span {
                            class: "text-sm text-gray-800 font-mono",
                            "{*limit.read()} events"
                        }
                    }
                    input {
                        r#type: "range",
                        min: "100",
                        max: "2000",
                        step: "100",
                        value: "{*limit.read()}",
                        class: "w-full",
                        oninput: {
                            let mut limit = limit.clone();
                            move |ev| {
                                if let Ok(val) = ev.value().parse::<usize>() {
                                    *limit.write() = val;
                                }
                            }
                        }
                    }
                    div {
                        class: "flex justify-between text-xs text-gray-500",
                        span { "100" }
                        span { "2000" }
                    }
                }
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
                class: "flex items-center gap-2 py-2 hover:bg-gray-50 rounded px-2 flex-wrap",
                button {
                    class: "text-gray-400 hover:text-gray-600 flex-shrink-0",
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
                // Display location in header
                if let Some(ref location) = span.location {
                    if !location.is_empty() {
                        span {
                            class: "text-xs px-2 py-0.5 bg-gray-100 text-gray-700 rounded font-mono",
                            "{location}"
                        }
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
                    // Attributes - displayed first
                    if let Some(ref attrs) = span.attributes {
                        if !attrs.is_empty() {
                            div {
                                class: "text-xs text-gray-500 mb-1",
                                "Attributes:"
                            }
                            // Try to parse and format JSON attributes
                            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(attrs) {
                                if let Some(obj) = parsed.as_object() {
                                    div {
                                        class: "bg-gray-50 p-2 rounded mt-1 space-y-1",
                                        for (key, val) in obj.iter() {
                                            div {
                                                class: "flex items-start gap-2 text-xs",
                                                span {
                                                    class: "font-semibold text-gray-700 min-w-[100px]",
                                                    "{key}:"
                                                }
                                                span {
                                                    class: "text-gray-600 font-mono break-all",
                                                    {
                                                        match val {
                                                            serde_json::Value::String(s) => s.clone(),
                                                            serde_json::Value::Number(n) => n.to_string(),
                                                            serde_json::Value::Bool(b) => b.to_string(),
                                                            serde_json::Value::Null => "null".to_string(),
                                                            _ => format!("{}", val),
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                } else {
                                    div {
                                        class: "text-xs font-mono bg-gray-50 p-2 rounded mt-1 break-all",
                                        "{attrs}"
                                    }
                                }
                            } else {
                                div {
                                    class: "text-xs font-mono bg-gray-50 p-2 rounded mt-1 break-all",
                                    "{attrs}"
                                }
                            }
                        }
                    }
                    
                    // Events
                    if !span.events.is_empty() {
                        div {
                            class: "text-xs text-gray-500 mb-1 mt-2",
                            "Events ({span.events.len()}):"
                        }
                        for event in span.events.iter() {
                            EventView { event: event.clone() }
                        }
                    }
                    
                    // Children spans
                    if !span.children.is_empty() {
                        div {
                            class: "text-xs text-gray-500 mb-1 mt-2",
                            "Child Spans ({span.children.len()}):"
                        }
                        for child in span.children.iter() {
                            SpanView { span: child.clone(), depth: depth + 1 }
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

