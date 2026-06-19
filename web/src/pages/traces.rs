use dioxus::prelude::*;

use crate::api::{ApiClient, EventInfo, SpanInfo};
use crate::components::card::Card;
use crate::components::colors::colors;
use crate::components::common::{query_result, AsyncBoundary, EmptyState};
use crate::components::icon::Icon;
use crate::components::page::{PageContainer, PageTitle};
use crate::hooks::use_app_resource;

const DEFAULT_LIMIT: usize = 400;
const MIN_LIMIT: usize = 100;
const MAX_LIMIT: usize = 2000;
const LIMIT_STEP: usize = 100;

#[component]
pub fn Traces() -> Element {
    let limit = use_signal(|| DEFAULT_LIMIT);
    let filter = use_signal(String::new);
    let expand_all = use_signal(|| 0u32);
    let collapse_all = use_signal(|| 0u32);

    rsx! {
        PageContainer {
            PageTitle {
                title: "Traces".to_string(),
                subtitle: Some("Hierarchical span tree from python.trace_event".to_string()),
                icon: Some(&icondata::AiApiOutlined),
            }

            Card {
                title: "Span Tree",
                content_class: Some("p-0"),
                header_right: Some(rsx! {
                    TraceToolbar {
                        limit,
                        filter,
                        expand_all,
                        collapse_all,
                    }
                }),
                AsyncBoundary {
                    message: Some("Loading trace data…".to_string()),
                    TraceTreePanel {
                        limit,
                        filter,
                        expand_all,
                        collapse_all,
                    }
                }
            }
        }
    }
}

#[component]
fn TraceToolbar(
    limit: Signal<usize>,
    filter: Signal<String>,
    expand_all: Signal<u32>,
    collapse_all: Signal<u32>,
) -> Element {
    rsx! {
        div { class: "flex flex-wrap items-center gap-2 max-w-xl",
            div { class: "relative min-w-[140px] flex-1",
                span { class: "absolute left-2 top-1/2 -translate-y-1/2 text-gray-400 pointer-events-none",
                    Icon { icon: &icondata::AiSearchOutlined, class: "w-3.5 h-3.5" }
                }
                input {
                    r#type: "text",
                    class: "w-full pl-7 pr-2 py-1.5 text-xs rounded-md border border-gray-300 bg-white focus:outline-none focus:ring-2 focus:ring-blue-500/30 focus:border-blue-500",
                    placeholder: "Filter spans…",
                    value: "{filter}",
                    oninput: move |ev| filter.set(ev.value()),
                }
            }
            button {
                class: "px-2 py-1.5 text-xs rounded-md border border-gray-300 bg-white hover:bg-gray-50",
                title: "Expand all spans",
                onclick: move |_| expand_all.set(expand_all() + 1),
                "Expand"
            }
            button {
                class: "px-2 py-1.5 text-xs rounded-md border border-gray-300 bg-white hover:bg-gray-50",
                title: "Collapse all spans",
                onclick: move |_| collapse_all.set(collapse_all() + 1),
                "Collapse"
            }
            div { class: "flex items-center gap-2 pl-1 border-l border-gray-200",
                span { class: "text-xs text-gray-500 whitespace-nowrap font-mono", "{limit()} evt" }
                input {
                    r#type: "range",
                    min: "{MIN_LIMIT}",
                    max: "{MAX_LIMIT}",
                    step: "{LIMIT_STEP}",
                    value: "{limit}",
                    class: "w-24 accent-blue-600",
                    oninput: move |ev| {
                        if let Ok(val) = ev.value().parse::<usize>() {
                            limit.set(val);
                        }
                    },
                }
            }
        }
    }
}

#[component]
fn TraceTreePanel(
    limit: Signal<usize>,
    filter: Signal<String>,
    expand_all: Signal<u32>,
    collapse_all: Signal<u32>,
) -> Element {
    let spans = use_app_resource(move || {
        let limit_val = limit();
        async move { ApiClient::new().get_span_tree(Some(limit_val)).await }
    });
    let tree = spans.suspend()?();

    query_result(
        tree,
        |spans| spans.is_empty(),
        "No trace data available. Start tracing with probing.tracing.span() or TorchProbe spans.",
        move |spans| {
            let filtered = filter_span_tree(&spans, &filter());
            let total = count_spans(&spans);
            let roots = spans.len();
            let shown = count_spans(&filtered);
            rsx! {
                div { class: "border-b border-gray-200 px-4 py-2 bg-gray-50/80 flex flex-wrap items-center gap-x-3 gap-y-0.5 text-xs text-gray-600",
                    span { class: "font-medium text-gray-800", "{roots} roots" }
                    span { "·" }
                    span { "{total} spans" }
                    span { "·" }
                    span { "limit {limit()}" }
                    if !filter.read().trim().is_empty() {
                        span { "·" }
                        span { class: "text-blue-700", "{shown} matched" }
                    }
                }
                if filtered.is_empty() {
                    div { class: "px-4 py-10",
                        EmptyState { message: format!("No spans match \"{}\"", filter()) }
                    }
                } else {
                    div { class: "px-2 py-2 max-h-[calc(100vh-14rem)] overflow-y-auto font-mono text-xs leading-5",
                        for span in filtered {
                            SpanView {
                                key: "{span.span_id}",
                                span: span.clone(),
                                depth: 0,
                                expand_all,
                                collapse_all,
                            }
                        }
                    }
                }
            }
        },
    )
}

fn count_spans(spans: &[SpanInfo]) -> usize {
    spans
        .iter()
        .map(|s| 1 + count_spans(&s.children))
        .sum()
}

fn filter_span_tree(spans: &[SpanInfo], query: &str) -> Vec<SpanInfo> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return spans.to_vec();
    }
    spans
        .iter()
        .filter_map(|span| {
            let children = filter_span_tree(&span.children, query);
            let name_match = span.name.to_lowercase().contains(&q)
                || span
                    .kind
                    .as_ref()
                    .is_some_and(|k| k.to_lowercase().contains(&q))
                || span
                    .location
                    .as_ref()
                    .is_some_and(|l| l.to_lowercase().contains(&q));
            if name_match || !children.is_empty() {
                Some(SpanInfo {
                    children,
                    ..span.clone()
                })
            } else {
                None
            }
        })
        .collect()
}

fn span_duration_secs(span: &SpanInfo) -> Option<f64> {
    span.end_timestamp.map(|end| (end - span.start_timestamp) as f64 / 1_000_000_000.0)
}

fn duration_label(duration: f64) -> String {
    if duration >= 1.0 {
        format!("{duration:.3}s")
    } else if duration >= 0.001 {
        format!("{:.1}ms", duration * 1000.0)
    } else {
        format!("{:.0}us", duration * 1_000_000.0)
    }
}

#[component]
fn SpanView(
    span: SpanInfo,
    depth: usize,
    expand_all: Signal<u32>,
    collapse_all: Signal<u32>,
) -> Element {
    let mut expanded = use_signal(|| depth < 2);
    let has_children = !span.children.is_empty();
    let has_events = !span.events.is_empty();
    let has_attrs = span
        .attributes
        .as_ref()
        .is_some_and(|a| !a.trim().is_empty());
    let has_details = has_children || has_events || has_attrs;
    let duration = span_duration_secs(&span);
    let indent = depth * 20;

    use_effect(move || {
        if expand_all() > 0 {
            expanded.set(true);
        }
    });
    use_effect(move || {
        if collapse_all() > 0 {
            expanded.set(false);
        }
    });

    rsx! {
        div { class: "min-w-0",
            div {
                class: "group flex flex-wrap items-center gap-x-2 gap-y-0.5 py-0.5 px-1 rounded hover:bg-gray-50/90",
                style: if indent > 0 { format!("padding-left: {indent}px") } else { String::new() },
                if has_details {
                    button {
                        class: "shrink-0 w-4 h-4 flex items-center justify-center text-gray-400 hover:text-gray-700",
                        onclick: move |_| expanded.set(!expanded()),
                        if expanded() {
                            Icon { icon: &icondata::AiCaretDownOutlined, class: "w-3 h-3" }
                        } else {
                            Icon { icon: &icondata::AiCaretRightOutlined, class: "w-3 h-3" }
                        }
                    }
                } else {
                    span { class: "w-4 shrink-0" }
                }
                span { class: "font-semibold text-gray-900 shrink-0", "{span.name}" }
                if let Some(ref kind) = span.kind {
                    span {
                        class: format!(
                            "shrink-0 px-1.5 py-px rounded text-[10px] font-sans font-medium bg-{} text-{}",
                            colors::CONTENT_ACCENT_BG,
                            colors::CONTENT_ACCENT_TEXT,
                        ),
                        "{kind}"
                    }
                }
                if let Some(ref location) = span.location {
                    if !location.is_empty() {
                        span { class: "text-gray-500 truncate max-w-[14rem]", "{location}" }
                    }
                }
                span { class: "text-gray-400 shrink-0", "id:{span.span_id}" }
                if let Some(parent) = span.parent_id {
                    span { class: "text-gray-400 shrink-0", "↑{parent}" }
                }
                span { class: "text-gray-400 shrink-0", "t:{span.thread_id}" }
                if let Some(dur) = duration {
                    span { class: "text-emerald-700 font-medium shrink-0", "{duration_label(dur)}" }
                } else {
                    span { class: "text-amber-600 shrink-0", "active" }
                }
                if has_events {
                    span { class: "text-gray-400 shrink-0", "{span.events.len()}evt" }
                }
                if has_children {
                    span { class: "text-gray-400 shrink-0", "{span.children.len()}↓" }
                }
            }

            if expanded() && has_details {
                div {
                    class: "space-y-0.5 pb-1",
                    style: format!("padding-left: {}px", indent + 20),
                    if has_attrs {
                        AttributesInline { raw: span.attributes.clone().unwrap_or_default() }
                    }
                    if has_events {
                        for event in span.events.iter() {
                            EventView { event: event.clone() }
                        }
                    }
                    if has_children {
                        for child in span.children.iter() {
                            SpanView {
                                key: "{child.span_id}",
                                span: child.clone(),
                                depth: depth + 1,
                                expand_all,
                                collapse_all,
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn AttributesInline(raw: String) -> Element {
    rsx! {
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&raw) {
            if let Some(obj) = parsed.as_object() {
                div { class: "flex flex-wrap items-center gap-x-3 gap-y-0.5 py-0.5 text-gray-600",
                    for (key, val) in obj.iter() {
                        span { class: "inline-flex items-baseline gap-1 max-w-full",
                            span { class: "text-gray-500 shrink-0", "{key}:" }
                            span { class: "text-gray-800 break-all", { attribute_value(val) } }
                        }
                    }
                }
            } else {
                MetaInline { text: raw }
            }
        } else {
            MetaInline { text: raw }
        }
    }
}

#[component]
fn MetaInline(text: String) -> Element {
    rsx! {
        div { class: "py-0.5 text-gray-600 break-all whitespace-pre-wrap", "{text}" }
    }
}

fn attribute_value(val: &serde_json::Value) -> String {
    match val {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Null => "null".to_string(),
        _ => val.to_string(),
    }
}

#[component]
fn EventView(event: EventInfo) -> Element {
    rsx! {
        div { class: "flex flex-wrap items-baseline gap-x-2 gap-y-0 py-0.5 text-gray-600",
            span { class: "text-blue-500 shrink-0", "●" }
            span { class: "text-gray-800", "{event.name}" }
            if let Some(ref attrs) = event.attributes {
                if !attrs.is_empty() {
                    span { class: "text-gray-500 break-all", "{attrs}" }
                }
            }
        }
    }
}
