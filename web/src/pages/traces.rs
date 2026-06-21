use dioxus::prelude::*;
use dioxus_router::Link;

use crate::api::{ApiClient, EventInfo, SpanInfo};
use crate::app::Route;
use crate::components::card::Card;
use crate::components::colors::colors;
use crate::components::common::{query_result, AsyncBoundary, EmptyState};
use crate::components::icon::Icon;
use crate::components::page::{PageContainer, PageTitle};
use crate::components::poll_status::{ManualRefreshStatus, RefreshButton};
use crate::hooks::use_app_resource;
use crate::state::investigation::{
    clear_spans_investigation_filters, investigation_context_key, set_trace_context,
    sync_spans_filters_to_context, InvestigationContext, INVESTIGATION_CONTEXT,
};
use crate::state::profiling::SPANS_TREE_LIMIT;

const SPANS_LIMIT_MIN: usize = 100;
const SPANS_LIMIT_MAX: usize = 5000;
const SPANS_LIMIT_STEP: usize = 100;

#[component]
pub fn Traces() -> Element {
    let mut refresh = use_signal(|| 0u32);
    let refresh_tick = refresh();
    let mut filter = use_signal(String::new);
    let expand_all = use_signal(|| 0u32);
    let collapse_all = use_signal(|| 0u32);
    let mut trace_id_filter = use_signal(String::new);
    let mut thread_filter = use_signal(String::new);
    let min_ms_filter = use_signal(String::new);
    let active_only = use_signal(|| false);
    let mut show_advanced = use_signal(|| false);
    let mut last_applied_ctx = use_signal(String::new);
    let clear_filters_tick = use_signal(|| 0u32);

    use_effect(move || {
        let ctx = INVESTIGATION_CONTEXT.read().clone();
        let key = investigation_context_key(&ctx);
        if key == last_applied_ctx() {
            return;
        }
        last_applied_ctx.set(key);
        trace_id_filter.set(ctx.trace_id.map(|t| t.to_string()).unwrap_or_default());
        // Apply thread filter when trace context is active, or when jumping from Dashboard
        // with thread-only context (no span_name / local_step — avoids Stacks page tid bleed).
        let thread_only = ctx.tid.is_some()
            && ctx.trace_id.is_none()
            && ctx.span_name.is_none()
            && ctx.local_step.is_none();
        filter.set(ctx.span_name.unwrap_or_default());
        if ctx.trace_id.is_some() || thread_only {
            thread_filter.set(ctx.tid.map(|t| t.to_string()).unwrap_or_default());
            show_advanced.set(true);
        } else {
            thread_filter.set(String::new());
            show_advanced.set(ctx.local_step.is_some());
        }
    });

    rsx! {
        PageContainer {
            PageTitle {
                title: "Spans".to_string(),
                subtitle: Some(
                    "Hierarchical tracing spans from python.trace_event. For chrome trace timelines, use Profiling → Chrome trace.".to_string(),
                ),
                icon: Some(&icondata::AiApiOutlined),
                header_right: Some(rsx! {
                    ManualRefreshStatus { refresh_tick }
                    RefreshButton {
                        onclick: move |_| refresh.set(refresh() + 1),
                    }
                }),
            }

            Card {
                title: "Span Tree",
                content_class: Some("p-0"),
                header_right: Some(rsx! {
                    TraceToolbar {
                        refresh,
                        filter,
                        expand_all,
                        collapse_all,
                        trace_id_filter,
                        thread_filter,
                        min_ms_filter,
                        active_only,
                        show_advanced,
                        clear_filters_tick,
                    }
                }),
                AsyncBoundary {
                    message: Some("Loading trace data…".to_string()),
                    TraceTreePanel {
                        refresh,
                        filter,
                        trace_id_filter,
                        thread_filter,
                        min_ms_filter,
                        active_only,
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
    refresh: Signal<u32>,
    filter: Signal<String>,
    expand_all: Signal<u32>,
    collapse_all: Signal<u32>,
    trace_id_filter: Signal<String>,
    thread_filter: Signal<String>,
    min_ms_filter: Signal<String>,
    active_only: Signal<bool>,
    show_advanced: Signal<bool>,
    clear_filters_tick: Signal<u32>,
) -> Element {
    let limit = *SPANS_TREE_LIMIT.read();
    let filters_active = {
        let _ = clear_filters_tick();
        !filter.read().trim().is_empty()
            || !trace_id_filter.read().trim().is_empty()
            || !thread_filter.read().trim().is_empty()
            || !min_ms_filter.read().trim().is_empty()
            || active_only()
            || {
                let ctx = INVESTIGATION_CONTEXT.read();
                ctx.tid.is_some()
                    || ctx.trace_id.is_some()
                    || ctx.span_name.is_some()
                    || ctx.local_step.is_some()
            }
    };
    rsx! {
        div { class: "flex flex-col gap-2 max-w-3xl w-full",
            div { class: "flex flex-wrap items-center gap-2",
                div { class: "relative min-w-[140px] flex-1",
                    span { class: "absolute left-2 top-1/2 -translate-y-1/2 text-gray-400 pointer-events-none",
                        Icon { icon: &icondata::AiSearchOutlined, class: "w-3.5 h-3.5" }
                    }
                    input {
                        r#type: "text",
                        class: "w-full pl-7 pr-2 py-1.5 text-xs rounded-md border border-gray-300 bg-white focus:outline-none focus:ring-2 focus:ring-blue-500/30 focus:border-blue-500",
                        placeholder: "Filter spans…",
                        value: "{filter}",
                        oninput: move |ev| {
                            let value = ev.value();
                            filter.set(value.clone());
                            sync_spans_filters_to_context(
                                &value,
                                &thread_filter.read(),
                                &trace_id_filter.read(),
                            );
                        },
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
                button {
                    class: if show_advanced() {
                        "px-2 py-1.5 text-xs rounded-md border border-blue-300 bg-blue-50 text-blue-700"
                    } else {
                        "px-2 py-1.5 text-xs rounded-md border border-gray-300 bg-white hover:bg-gray-50"
                    },
                    onclick: move |_| show_advanced.set(!show_advanced()),
                    "Filters"
                }
                if filters_active {
                    button {
                        class: "px-2 py-1.5 text-xs rounded-md border border-gray-300 bg-white hover:bg-gray-50 text-gray-700",
                        title: "Clear all span filters",
                        onclick: move |_| {
                            filter.set(String::new());
                            trace_id_filter.set(String::new());
                            thread_filter.set(String::new());
                            min_ms_filter.set(String::new());
                            active_only.set(false);
                            show_advanced.set(false);
                            clear_spans_investigation_filters();
                            clear_filters_tick.set(clear_filters_tick() + 1);
                        },
                        "Clear filters"
                    }
                }
                Link {
                    to: Route::ProfilingViewPage { view: "trace".to_string() },
                    class: format!(
                        "px-2 py-1.5 text-xs rounded-md border border-{} bg-{} text-{} hover:opacity-90 whitespace-nowrap",
                        colors::CONTENT_ACCENT_BORDER,
                        colors::CONTENT_ACCENT_BG,
                        colors::CONTENT_ACCENT_TEXT,
                    ),
                    title: "Open chrome trace event timeline under Profiling (not this span tree)",
                    "Chrome trace →"
                }
                div { class: "flex items-center gap-2 pl-1 border-l border-gray-200",
                    span { class: "text-xs text-gray-500 whitespace-nowrap font-mono", "{limit} rows" }
                    input {
                        r#type: "range",
                        min: "{SPANS_LIMIT_MIN}",
                        max: "{SPANS_LIMIT_MAX}",
                        step: "{SPANS_LIMIT_STEP}",
                        value: "{limit}",
                        class: "w-24 accent-blue-600",
                        title: "Max trace_event rows loaded for the span tree",
                        oninput: move |ev| {
                            if let Ok(val) = ev.value().parse::<usize>() {
                                *SPANS_TREE_LIMIT.write() = val;
                                refresh.set(refresh() + 1);
                            }
                        },
                    }
                }
            }
            if show_advanced() {
                div { class: "flex flex-wrap items-center gap-2",
                    input {
                        r#type: "text",
                        class: "w-28 px-2 py-1.5 text-xs rounded-md border border-gray-300 bg-white font-mono",
                        placeholder: "trace id",
                        value: "{trace_id_filter}",
                        oninput: move |ev| {
                            let value = ev.value();
                            trace_id_filter.set(value.clone());
                            sync_spans_filters_to_context(
                                &filter.read(),
                                &thread_filter.read(),
                                &value,
                            );
                        },
                    }
                    input {
                        r#type: "text",
                        class: "w-24 px-2 py-1.5 text-xs rounded-md border border-gray-300 bg-white font-mono",
                        placeholder: "thread",
                        value: "{thread_filter}",
                        oninput: move |ev| {
                            let value = ev.value();
                            thread_filter.set(value.clone());
                            sync_spans_filters_to_context(
                                &filter.read(),
                                &value,
                                &trace_id_filter.read(),
                            );
                        },
                    }
                    input {
                        r#type: "text",
                        class: "w-24 px-2 py-1.5 text-xs rounded-md border border-gray-300 bg-white font-mono",
                        placeholder: "min ms",
                        value: "{min_ms_filter}",
                        oninput: move |ev| min_ms_filter.set(ev.value()),
                    }
                    label { class: "inline-flex items-center gap-1.5 text-xs text-gray-600",
                        input {
                            r#type: "checkbox",
                            class: "rounded border-gray-300",
                            checked: active_only(),
                            onchange: move |ev| active_only.set(ev.checked()),
                        }
                        "Active only"
                    }
                }
            }
        }
    }
}

#[component]
fn TraceTreePanel(
    refresh: Signal<u32>,
    filter: Signal<String>,
    trace_id_filter: Signal<String>,
    thread_filter: Signal<String>,
    min_ms_filter: Signal<String>,
    active_only: Signal<bool>,
    expand_all: Signal<u32>,
    collapse_all: Signal<u32>,
) -> Element {
    let spans = use_app_resource(move || {
        let _ = refresh();
        let limit_val = *SPANS_TREE_LIMIT.read();
        async move { ApiClient::new().get_span_tree(Some(limit_val)).await }
    });
    let tree = spans.suspend()?();

    query_result(
        tree,
        |spans| spans.is_empty(),
        "No trace data available. Start tracing with probing.tracing.span() or TorchProbe spans.",
        move |spans| {
            let ctx = INVESTIGATION_CONTEXT.read().clone();
            let highlight = SpanHighlight::from_context(&ctx);
            let advanced = TraceAdvancedFilters {
                trace_id: trace_id_filter().trim().parse().ok(),
                thread_id: thread_filter().trim().parse().ok(),
                min_duration_ms: min_ms_filter().trim().parse().ok(),
                active_only: active_only(),
                local_step: ctx.local_step,
            };
            let filtered = filter_span_tree(&spans, &filter(), &advanced);
            let total = count_spans(&spans);
            let roots = spans.len();
            let shown = count_spans(&filtered);
            let limit_display = *SPANS_TREE_LIMIT.read();
            let filter_summary = active_filter_summary(&filter(), &advanced);
            rsx! {
                div { class: "border-b border-gray-200 px-4 py-2 bg-gray-50/80 flex flex-wrap items-center gap-x-3 gap-y-0.5 text-xs text-gray-600",
                    span { class: "font-medium text-gray-800", "{roots} roots" }
                    span { "·" }
                    span { "{total} spans" }
                    span { "·" }
                    span { "limit {limit_display}" }
                    if !ctx.is_empty() {
                        span { "·" }
                        span {
                            class: "text-blue-700",
                            title: "Filters sync with investigation context and URL",
                            "Context linked"
                        }
                    }
                    if !filter_summary.is_empty() {
                        span { "·" }
                        span { class: "text-blue-700", "{shown} matched · {filter_summary}" }
                    }
                }
                if filtered.is_empty() {
                    div { class: "px-4 py-10",
                        EmptyState {
                            message: if filter_summary.is_empty() {
                                "No spans in the current window.".to_string()
                            } else {
                                format!("No spans match {filter_summary}")
                            },
                        }
                    }
                } else {
                    div { class: "px-2 py-2 max-h-[calc(100vh-14rem)] overflow-y-auto font-mono text-xs leading-5",
                        for span in filtered {
                            SpanView {
                                key: "{span.span_id}",
                                span: span.clone(),
                                depth: 0,
                                highlight: highlight.clone(),
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
    spans.iter().map(|s| 1 + count_spans(&s.children)).sum()
}

fn filter_span_tree(
    spans: &[SpanInfo],
    query: &str,
    advanced: &TraceAdvancedFilters,
) -> Vec<SpanInfo> {
    spans
        .iter()
        .filter_map(|span| {
            let children = filter_span_tree(&span.children, query, advanced);
            let self_matches =
                span_matches_text(span, query) && span_matches_advanced(span, advanced);
            if self_matches || !children.is_empty() {
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

fn span_matches_text(span: &SpanInfo, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return true;
    }
    span.name.to_lowercase().contains(&q)
        || span
            .phase
            .as_ref()
            .is_some_and(|p| p.to_lowercase().contains(&q))
        || span
            .location
            .as_ref()
            .is_some_and(|l| l.to_lowercase().contains(&q))
        || span
            .attributes
            .as_ref()
            .is_some_and(|a| a.to_lowercase().contains(&q))
}

fn active_filter_summary(query: &str, advanced: &TraceAdvancedFilters) -> String {
    let mut parts = Vec::new();
    let q = query.trim();
    if !q.is_empty() {
        parts.push(format!("text:{q:?}"));
    }
    if let Some(trace_id) = advanced.trace_id {
        parts.push(format!("trace_id={trace_id}"));
    }
    if let Some(thread_id) = advanced.thread_id {
        parts.push(format!("thread={thread_id}"));
    }
    if let Some(step) = advanced.local_step {
        parts.push(format!("local_step={step}"));
    }
    if let Some(min_ms) = advanced.min_duration_ms {
        parts.push(format!("min_ms={min_ms}"));
    }
    if advanced.active_only {
        parts.push("active".to_string());
    }
    parts.join(", ")
}

struct TraceAdvancedFilters {
    trace_id: Option<i64>,
    thread_id: Option<i64>,
    min_duration_ms: Option<f64>,
    active_only: bool,
    local_step: Option<i64>,
}

fn span_local_step(span: &SpanInfo) -> Option<i64> {
    let raw = span.attributes.as_ref()?;
    let value: serde_json::Value = serde_json::from_str(raw).ok()?;
    value
        .get("local_step")
        .and_then(|v| v.as_i64().or_else(|| v.as_u64().map(|n| n as i64)))
}

fn span_matches_advanced(span: &SpanInfo, filters: &TraceAdvancedFilters) -> bool {
    if let Some(trace_id) = filters.trace_id {
        if span.trace_id != trace_id {
            return false;
        }
    }
    if let Some(thread_id) = filters.thread_id {
        if span.thread_id != thread_id {
            return false;
        }
    }
    if let Some(step) = filters.local_step {
        match span_local_step(span) {
            Some(s) if s == step => {}
            _ => return false,
        }
    }
    if filters.active_only && span.end_timestamp.is_some() {
        return false;
    }
    if let Some(min_ms) = filters.min_duration_ms {
        if let Some(end) = span.end_timestamp {
            let dur_ms = (end - span.start_timestamp) as f64 / 1_000_000.0;
            if dur_ms < min_ms {
                return false;
            }
        } else {
            return false;
        }
    }
    true
}

fn span_duration_secs(span: &SpanInfo) -> Option<f64> {
    span.end_timestamp
        .map(|end| (end - span.start_timestamp) as f64 / 1_000_000_000.0)
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

fn span_is_primary_selection(span: &SpanInfo, highlight: &SpanHighlight) -> bool {
    if let Some(trace_id) = highlight.trace_id {
        if span.trace_id != trace_id {
            return false;
        }
        return highlight
            .span_name
            .as_ref()
            .map(|n| n == &span.name)
            .unwrap_or(true);
    }
    false
}

fn span_matches_thread_only(span: &SpanInfo, highlight: &SpanHighlight) -> bool {
    highlight.trace_id.is_none() && highlight.tid == Some(span.thread_id as i32)
}

fn span_row_class(span: &SpanInfo, highlight: &SpanHighlight) -> String {
    let base =
        "group flex flex-wrap items-center gap-x-2 gap-y-0.5 py-0.5 px-1 rounded cursor-pointer";
    if span_is_primary_selection(span, highlight) {
        format!("{base} bg-blue-100 ring-1 ring-inset ring-blue-300 hover:bg-blue-100")
    } else if span_matches_thread_only(span, highlight) {
        format!("{base} bg-blue-50/70 hover:bg-blue-50")
    } else {
        format!("{base} hover:bg-gray-50/90")
    }
}

#[derive(Clone, PartialEq)]
struct SpanHighlight {
    trace_id: Option<i64>,
    tid: Option<i32>,
    span_name: Option<String>,
}

impl SpanHighlight {
    fn from_context(ctx: &InvestigationContext) -> Self {
        Self {
            trace_id: ctx.trace_id,
            tid: ctx.tid,
            span_name: ctx.span_name.clone(),
        }
    }
}

#[component]
fn SpanView(
    span: SpanInfo,
    depth: usize,
    highlight: SpanHighlight,
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
    let trace_id = span.trace_id;
    let thread_id = span.thread_id as i32;
    let span_name = span.name.clone();
    let row_class = span_row_class(&span, &highlight);

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
                class: "{row_class}",
                style: if indent > 0 { format!("padding-left: {indent}px") } else { String::new() },
                onclick: move |_| {
                    set_trace_context(trace_id, Some(&span_name), Some(thread_id));
                },
                if has_details {
                    button {
                        class: "shrink-0 w-4 h-4 flex items-center justify-center text-gray-400 hover:text-gray-700",
                        onclick: move |e| {
                            e.stop_propagation();
                            expanded.set(!expanded());
                        },
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
                if let Some(ref phase) = span.phase {
                    span {
                        class: format!(
                            "shrink-0 px-1.5 py-px rounded text-[10px] font-sans font-medium bg-{} text-{}",
                            colors::CONTENT_ACCENT_BG,
                            colors::CONTENT_ACCENT_TEXT,
                        ),
                        "{phase}"
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
                                highlight: highlight.clone(),
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
