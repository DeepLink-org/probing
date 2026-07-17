use dioxus::prelude::*;
use dioxus_router::use_navigator;

use crate::app::Route;
use crate::components::card::Card;
use crate::components::colors::colors;
use crate::components::page::{PageContainer, PageTitle};
use crate::components::common::{LoadingState, ErrorState};
use crate::hooks::use_api_simple;
use crate::api::{ApiClient, SpanInfo, EventInfo, TraceProcessInfo};
use crate::rl_contract::{
    self, is_rollout_submit_parent_span,
    is_step_parent_span, is_train_timeline_span, is_rollout_worker_role, logical_step_key,
    LogicalStepKey, ROLLOUT_TIMELINE_EMPTY_HINT, TRAIN_TIMELINE_EMPTY_HINT,
};
use crate::state::rl::{
    estimate_detail_panel_height, ROLLOUT_FILTER, ROLLOUT_FILTER_INPUT, RL_DETAIL_PANEL_HEIGHT,
    RL_DETAIL_PANEL_HEIGHT_DEFAULT, RL_DETAIL_PANEL_HEIGHT_MAX, RL_DETAIL_PANEL_HEIGHT_MIN,
};
use crate::utils::tracing_viewer;
use std::collections::{HashMap, HashSet};

/// RL observability view selected from the sidebar route.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RlViewMode {
    Rollout,
    Train,
    Spans,
    ProcessTimeline,
    Perfetto,
}

impl RlViewMode {
    pub fn title(self) -> &'static str {
        match self {
            Self::Rollout => "Rollout",
            Self::Train => "Train",
            Self::Spans => "Spans",
            Self::ProcessTimeline => "Process Timeline",
            Self::Perfetto => "Perfetto",
        }
    }

    pub fn subtitle(self) -> &'static str {
        match self {
            Self::Rollout => "Per-trajectory phase timing across rollout workers",
            Self::Train => "Training batch phases keyed by train step",
            Self::Spans => "Distributed span hierarchy with cross-process linking",
            Self::ProcessTimeline => "Per-process span timing and batch drill-down",
            Self::Perfetto => "Chrome trace export for the loaded span set",
        }
    }

    pub fn icon(self) -> &'static icondata::Icon {
        match self {
            Self::Rollout => &icondata::AiDeploymentUnitOutlined,
            Self::Train => &icondata::AiLineChartOutlined,
            Self::Spans => &icondata::AiApartmentOutlined,
            Self::ProcessTimeline => &icondata::AiClockCircleOutlined,
            Self::Perfetto => &icondata::AiThunderboltOutlined,
        }
    }

    fn internal_key(self) -> &'static str {
        match self {
            Self::Rollout => "rl",
            Self::Train => "train",
            Self::Spans => "tree",
            Self::ProcessTimeline => "profiling",
            Self::Perfetto => "rollout-perfetto",
        }
    }

    fn shows_rollout_filter(self) -> bool {
        matches!(self, Self::Rollout)
    }

    fn uses_rollout_filter(self) -> bool {
        matches!(self, Self::Rollout | Self::Perfetto)
    }
}

#[component]
pub fn RlObservability(view: RlViewMode) -> Element {
    let is_perfetto = view == RlViewMode::Perfetto;
    let limit = use_signal(|| if is_perfetto { 2000 } else { 400 });
    let timeline_depth = use_signal(|| 2usize);
    let selected_batch_key = use_signal(|| String::new());
    let process_filter = use_signal(|| {
        if is_perfetto {
            "all".to_string()
        } else {
            "driver".to_string()
        }
    });
    let function_filter = use_signal(|| String::new());
    let discovered_processes = use_signal(|| Vec::<TraceProcessInfo>::new());
    let state = use_api_simple::<Vec<SpanInfo>>();
    let navigator = use_navigator();
    let view_key = view.internal_key();

    let shows_rollout_filter = view.shows_rollout_filter();
    let uses_rollout_filter = view.uses_rollout_filter();

    // Create dependency, recalculate when limit or rollout filter changes.
    let data_query = use_memo({
        let limit = limit.clone();
        move || {
            let rollout_id = if uses_rollout_filter {
                ROLLOUT_FILTER.read().trim().to_string()
            } else {
                String::new()
            };
            (*limit.read(), rollout_id)
        }
    });
    // Refetch data when limit or rollout filter changes.
    use_effect({
        let data_query = data_query.clone();
        let mut loading = state.loading;
        let mut data = state.data;
        let mut discovered_processes = discovered_processes.clone();
        move || {
            let (limit_val, rollout_id) = data_query.read().clone();
            spawn(async move {
                *loading.write() = true;
                let client = ApiClient::new();
                let result = async {
                    let processes = client.get_trace_processes().await.unwrap_or_default();
                    *discovered_processes.write() = processes.clone();

                    let mut spans = if rollout_id.is_empty() {
                        let fetch_limit = expanded_trace_fetch_limit(limit_val);
                        let mut spans = client.get_span_tree(Some(fetch_limit)).await?;
                        let local_pids = collect_span_process_pids(&spans);
                        for process in processes {
                            if local_pids.contains(&process.pid) {
                                continue;
                            }
                            if let Ok(mut process_spans) = client.get_span_tree_for_pid(process.pid, Some(fetch_limit)).await {
                                spans.append(&mut process_spans);
                            }
                        }
                        spans
                    } else {
                        let mut spans = client.get_span_tree_for_rollout_id(&rollout_id).await?;
                        let local_pids = collect_span_process_pids(&spans);
                        for process in processes {
                            if local_pids.contains(&process.pid) {
                                continue;
                            }
                            if let Ok(mut process_spans) = client.get_span_tree_for_pid_and_rollout_id(process.pid, &rollout_id).await {
                                spans.append(&mut process_spans);
                            }
                        }
                        spans
                    };
                    spans = link_cross_process_spans(spans);
                    spans.sort_by_key(|span| span.start_timestamp);
                    Ok(spans)
                }
                .await;
                *data.write() = Some(result);
                *loading.write() = false;
            });
        }
    });

    let display_fetch_limit = expanded_trace_fetch_limit(*limit.read());
    let active_rollout_filter = if shows_rollout_filter {
        ROLLOUT_FILTER.read().trim().to_string()
    } else {
        String::new()
    };
    let rollout_filter_input_value = if shows_rollout_filter {
        ROLLOUT_FILTER_INPUT.read().clone()
    } else {
        String::new()
    };

    if is_perfetto {
        return rsx! {
            div {
                class: "flex h-full min-h-0 flex-col bg-white",
                if state.is_loading() {
                    div {
                        class: "flex flex-1 items-center justify-center text-sm text-gray-500",
                        "Loading trace data..."
                    }
                } else if let Some(Ok(spans)) = state.data.read().as_ref() {
                    if spans.is_empty() {
                        div {
                            class: "flex flex-1 items-center justify-center p-8 text-center text-sm text-gray-500",
                            "No trace data available. Start tracing with probing.tracing.span()"
                        }
                    } else {
                        {
                            let all_display_spans = build_display_spans(spans);
                            let display_spans = filter_display_spans(
                                &all_display_spans,
                                "all",
                                "",
                            );
                            rsx! {
                                RolloutPerfettoTimeline {
                                    spans: display_spans,
                                    fullscreen: true,
                                }
                            }
                        }
                    }
                } else if let Some(Err(err)) = state.data.read().as_ref() {
                    div {
                        class: "flex flex-1 items-center justify-center p-8",
                        ErrorState { error: err.display_message(), title: None }
                    }
                }
            }
        };
    }

    rsx! {
        PageContainer {
            PageTitle {
                title: view.title().to_string(),
                subtitle: Some(view.subtitle().to_string()),
                icon: Some(view.icon()),
            }
            // Limit control slider
            Card {
                title: "Context",
                    div {
                        class: "space-y-2",
                        if shows_rollout_filter {
                            div {
                                class: "flex flex-wrap items-center gap-2 rounded border border-blue-100 bg-blue-50 px-3 py-2",
                                span {
                                    class: "text-sm text-gray-700",
                                    "Rollout ID"
                                }
                                input {
                                    r#type: "text",
                                    value: "{rollout_filter_input_value}",
                                    placeholder: "e.g. 12",
                                    class: "w-40 rounded border border-gray-200 bg-white px-2 py-1 text-sm font-mono",
                                    oninput: move |ev| *ROLLOUT_FILTER_INPUT.write() = ev.value()
                                }
                                button {
                                    class: format!("rounded px-3 py-1.5 text-sm bg-{} text-white", colors::PRIMARY),
                                    onclick: {
                                        let mut selected_batch_key = selected_batch_key.clone();
                                        let mut process_filter = process_filter.clone();
                                        let nav = navigator.clone();
                                        move |_| {
                                            *ROLLOUT_FILTER.write() =
                                                ROLLOUT_FILTER_INPUT.read().trim().to_string();
                                            *selected_batch_key.write() = String::new();
                                            *process_filter.write() = "all".to_string();
                                            nav.push(Route::RolloutPage {});
                                        }
                                    },
                                    "Load rollout"
                                }
                                button {
                                    class: "rounded bg-white px-3 py-1.5 text-sm text-gray-700 hover:bg-gray-100",
                                    onclick: {
                                        let mut selected_batch_key = selected_batch_key.clone();
                                        move |_| {
                                            *ROLLOUT_FILTER_INPUT.write() = String::new();
                                            *ROLLOUT_FILTER.write() = String::new();
                                            *selected_batch_key.write() = String::new();
                                        }
                                    },
                                    "Clear"
                                }
                                span {
                                    class: "text-xs text-gray-600",
                                    if active_rollout_filter.is_empty() {
                                        "Empty means latest-event mode."
                                    } else {
                                        "Fetching all spans tagged with rollout_id={active_rollout_filter} across processes."
                                    }
                                }
                            }
                        }
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
                        div {
                            class: "text-xs text-gray-500",
                            if shows_rollout_filter && active_rollout_filter.is_empty() {
                                "Trace linking fetches up to {display_fetch_limit} events per process to keep cross-process parents in range."
                            } else if shows_rollout_filter {
                                "Rollout ID mode ignores the event limit and fetches all matching spans for the selected rollout."
                            } else {
                                "Fetches up to {display_fetch_limit} events per process."
                            }
                        }
                }
            }

            Card {
                title: "Trace Data",
                if state.is_loading() {
                    LoadingState { message: Some("Loading trace data...".to_string()) }
                } else if let Some(Ok(spans)) = state.data.read().as_ref() {
                    if spans.is_empty() {
                        div {
                            class: "text-center py-8 text-gray-500",
                            "No trace data available. Start tracing with probing.tracing.span()"
                        }
                    } else {
                        {
                            let all_display_spans = build_display_spans(spans);
                            let process_options = collect_process_options(&all_display_spans, &discovered_processes.read());
                            let function_options = collect_function_options(&all_display_spans);
                            let selected_process_filter = process_filter.read().clone();
                            let selected_function_filter = function_filter.read().clone();
                            let display_spans = filter_display_spans(
                                &all_display_spans,
                                &selected_process_filter,
                                &selected_function_filter,
                            );
                            let discovered_count = discovered_processes.read().len();
                            let batch_options = collect_batch_options(&display_spans);
                            let current_batch_key = selected_batch_key.read().clone();
                            let selected_batch_key_for_view = if batch_options.iter().any(|batch| batch.unique_key == current_batch_key) {
                                current_batch_key
                            } else {
                                String::new()
                            };
                            let selected_batch_label = batch_options
                                .iter()
                                .find(|batch| batch.unique_key == selected_batch_key_for_view)
                                .map(|batch| batch.display_id.clone())
                                .unwrap_or_else(|| "All batches".to_string());
                            let depth_value = *timeline_depth.read();
                            let link_summary = cross_process_link_summary_view(&display_spans);

                            rsx! {
                                div {
                                    class: "space-y-6",
                                    div {
                                        class: "rounded border border-gray-100 bg-gray-50 px-3 py-3",
                                        div {
                                            class: "flex flex-wrap items-center gap-4",
                                            div {
                                                class: "flex items-center gap-2",
                                                span { class: "text-sm text-gray-600", "Process" }
                                                select {
                                                    class: "max-w-[360px] rounded border border-gray-200 bg-white px-2 py-1 text-sm",
                                                    value: "{selected_process_filter}",
                                                    oninput: {
                                                        let mut process_filter = process_filter.clone();
                                                        let mut selected_batch_key = selected_batch_key.clone();
                                                        move |ev| {
                                                            *process_filter.write() = ev.value();
                                                            *selected_batch_key.write() = String::new();
                                                        }
                                                    },
                                                    option { value: "driver", "Driver (default)" }
                                                    option { value: "all", "All processes with spans" }
                                                    for process_option in process_options.iter() {
                                                        option {
                                                            value: "{process_option.filter_key}",
                                                            "{process_option.label}"
                                                        }
                                                    }
                                                }
                                            }
                                            div {
                                                class: "flex items-center gap-2",
                                                span { class: "text-sm text-gray-600", "Function" }
                                                select {
                                                    class: "max-w-[360px] rounded border border-gray-200 bg-white px-2 py-1 text-sm",
                                                    value: "{selected_function_filter}",
                                                    oninput: {
                                                        let mut function_filter = function_filter.clone();
                                                        let mut selected_batch_key = selected_batch_key.clone();
                                                        move |ev| {
                                                            *function_filter.write() = ev.value();
                                                            *selected_batch_key.write() = String::new();
                                                        }
                                                    },
                                                    option { value: "", "All probing spans" }
                                                    for function_option in function_options.iter() {
                                                        option {
                                                            value: "{function_option.filter_key}",
                                                            "{function_option.label}"
                                                        }
                                                    }
                                                }
                                            }
                                            span {
                                                class: "text-xs text-gray-500",
                                                "discovered {discovered_count} Ray processes, showing {display_spans.len()} roots"
                                            }
                                        }
                                    }
                                    div {
                                        class: "rounded border border-gray-100 bg-gray-50 px-3 py-2 text-xs font-mono text-gray-600",
                                        "Cross Process Link: linked {link_summary.linked}, orphan {link_summary.orphan}, rollout parents {link_summary.parents}"
                                        if !link_summary.orphan_keys.is_empty() {
                                            span {
                                                class: "ml-2 text-gray-500",
                                                "orphan keys: {link_summary.orphan_keys}"
                                            }
                                        }
                                    }
                                    if view_key == "profiling" {
                                        div {
                                            class: "flex flex-wrap items-center gap-4 rounded border border-gray-100 bg-gray-50 px-3 py-2",
                                            div {
                                                class: "flex items-center gap-2",
                                                span { class: "text-sm text-gray-600", "Depth" }
                                                select {
                                                    class: "rounded border border-gray-200 bg-white px-2 py-1 text-sm",
                                                    value: "{depth_value}",
                                                    oninput: {
                                                        let mut timeline_depth = timeline_depth.clone();
                                                        move |ev| {
                                                            if let Ok(val) = ev.value().parse::<usize>() {
                                                                *timeline_depth.write() = val;
                                                            }
                                                        }
                                                    },
                                                    option { value: "1", "Level 1" }
                                                    option { value: "2", "Level 2" }
                                                    option { value: "3", "Level 3" }
                                                    option { value: "99", "All" }
                                                }
                                            }
                                            div {
                                                class: "flex items-center gap-2",
                                                span { class: "text-sm text-gray-600", "Batch" }
                                                select {
                                                    class: "max-w-[320px] rounded border border-gray-200 bg-white px-2 py-1 text-sm",
                                                    value: "{selected_batch_key_for_view}",
                                                    oninput: {
                                                        let mut selected_batch_key = selected_batch_key.clone();
                                                        move |ev| *selected_batch_key.write() = ev.value()
                                                    },
                                                    option { value: "", "All batches" }
                                                    for batch in batch_options.iter() {
                                                        option {
                                                            value: "{batch.unique_key}",
                                                            "{batch.display_id} | {batch.process_label} | span_id:{batch.span_id}"
                                                        }
                                                    }
                                                }
                                            }
                                            span {
                                                class: "text-xs text-gray-500",
                                                "selected: {selected_batch_label}"
                                            }
                                        }
                                        SpanTimeline {
                                            spans: display_spans.clone(),
                                            max_depth: depth_value,
                                            selected_batch_key: selected_batch_key_for_view.clone(),
                                        }
                                    } else if view_key == "rl" {
                                        RlTimeline {
                                            spans: display_spans.clone(),
                                        }
                                    } else if view_key == "rollout-perfetto" {
                                        RolloutPerfettoTimeline {
                                            spans: display_spans.clone(),
                                        }
                                    } else if view_key == "train" {
                                        TrainTimeline {
                                            spans: display_spans.clone(),
                                        }
                                    } else {
                                        div {
                                            class: "space-y-4",
                                            for span in display_spans.iter() {
                                                SpanView { span: span.clone(), depth: 0 }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                } else if let Some(Err(err)) = state.data.read().as_ref() {
                    ErrorState { error: err.display_message(), title: None }
                }
            }
        }
    }
}

#[allow(dead_code)]
#[component]
fn _unused_legacy_traces() -> Element {
    rsx! {
        RlObservability { view: RlViewMode::Rollout }
    }
}

#[derive(Clone, PartialEq)]
struct DisplaySpan {
    unique_key: String,
    span_id: i64,
    trace_id: i64,
    parent_id: Option<i64>,
    name: String,
    display_id: String,
    start_timestamp: i64,
    end_timestamp: Option<i64>,
    thread_id: i64,
    kind: Option<String>,
    location: Option<String>,
    attributes: Option<String>,
    process_label: String,
    children: Vec<DisplaySpan>,
    events: Vec<EventInfo>,
}

#[derive(Clone, PartialEq)]
struct FilterOption {
    filter_key: String,
    label: String,
}

#[component]
fn SpanView(span: DisplaySpan, depth: usize) -> Element {
    let indent = depth * 24;
    let duration = span.end_timestamp
        .map(|end| format_duration_ns(end - span.start_timestamp))
        .unwrap_or_else(|| "running".to_string());

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
                span {
                    class: "text-xs px-2 py-0.5 bg-gray-100 text-gray-600 rounded font-mono",
                    "{span.display_id}"
                }
                span {
                    class: "text-xs px-2 py-0.5 bg-gray-100 text-gray-600 rounded font-mono",
                    "{span.process_label}"
                }
                if let Some(ref kind) = span.kind {
                    span {
                        class: format!("text-xs px-2 py-0.5 bg-{} text-{} rounded", colors::CONTENT_ACCENT_BG, colors::CONTENT_ACCENT_TEXT),
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
                    "{duration}"
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

#[derive(Clone, PartialEq)]
struct TimelineSpan {
    unique_key: String,
    name: String,
    display_id: String,
    span_id: i64,
    kind: Option<String>,
    location: Option<String>,
    start: i64,
    duration: i64,
    depth: usize,
    process_label: String,
}

#[derive(Clone, Default)]
struct CrossProcessLinkSummaryView {
    linked: usize,
    orphan: usize,
    parents: usize,
    orphan_keys: String,
}

#[component]
fn SpanTimeline(spans: Vec<DisplaySpan>, max_depth: usize, selected_batch_key: String) -> Element {
    let rows = build_timeline_spans(&spans, max_depth, &selected_batch_key);
    if rows.is_empty() {
        return rsx! {
            div {
                class: "text-sm text-gray-500",
                "No completed spans available for timeline."
            }
        };
    }

    let min_start = rows.iter().map(|row| row.start).min().unwrap_or(0);
    let max_end = rows
        .iter()
        .map(|row| row.start + row.duration)
        .max()
        .unwrap_or(min_start + 1);
    let window = (max_end - min_start).max(1);
    let window_f = window as f64;
    let window_label = format_duration_ns(window);

    rsx! {
        div {
            class: "border-b border-gray-100 pb-4",
            div {
                class: "mb-3 flex items-center justify-between gap-4",
                h4 {
                    class: "text-sm font-semibold text-gray-900",
                    "Timeline"
                }
                span {
                    class: "text-xs font-mono text-gray-500",
                    "window {window_label}"
                }
            }
            div {
                class: "space-y-1",
                for row in rows.iter() {
                    {
                        let left = ((row.start - min_start) as f64 / window_f * 100.0).clamp(0.0, 100.0);
                        let width = (row.duration as f64 / window_f * 100.0)
                            .max(0.5)
                            .min((100.0 - left).max(0.5));
                        let start_offset = row.start - min_start;
                        let end_offset = start_offset + row.duration;
                        let start_label = format_duration_ns(start_offset);
                        let end_label = format_duration_ns(end_offset);
                        let duration_label = format_duration_ns(row.duration);
                        let tooltip = format!(
                            "{} ({})\nprocess: {}\nspan_id: {}\nstart: {}\nend: {}\nduration: {}",
                            row.name, row.display_id, row.process_label, row.span_id, start_label, end_label, duration_label,
                        );
                        rsx! {
                            div {
                                class: "grid grid-cols-[minmax(220px,34%)_1fr_auto] items-center gap-3 text-sm",
                                title: "{tooltip}",
                                div {
                                    class: "min-w-0",
                                    style: "padding-left: {row.depth.min(8) * 14}px",
                                    div {
                                        class: "flex items-center gap-2 min-w-0",
                                        span {
                                            class: "font-medium text-gray-900 truncate",
                                            "{row.name}"
                                        }
                                        span {
                                            class: "text-xs px-1.5 py-0.5 bg-gray-100 text-gray-600 rounded font-mono flex-shrink-0",
                                            "{row.display_id}"
                                        }
                                        span {
                                            class: "text-xs px-1.5 py-0.5 bg-gray-100 text-gray-600 rounded font-mono flex-shrink-0",
                                            "{row.process_label}"
                                        }
                                        if let Some(ref kind) = row.kind {
                                            span {
                                                class: format!("text-xs px-1.5 py-0.5 bg-{} text-{} rounded flex-shrink-0", colors::CONTENT_ACCENT_BG, colors::CONTENT_ACCENT_TEXT),
                                                "{kind}"
                                            }
                                        }
                                    }
                                    if let Some(ref location) = row.location {
                                        if !location.is_empty() {
                                            div {
                                                class: "mt-0.5 text-xs font-mono text-gray-400 truncate",
                                                "{location}"
                                            }
                                        }
                                    }
                                }
                                div {
                                    class: "relative h-5 rounded bg-gray-100 overflow-hidden",
                                    div {
                                        class: "absolute top-1 bottom-1 rounded bg-blue-500",
                                        style: "left: {left:.2}%; width: {width:.2}%;",
                                    }
                                }
                                div {
                                    class: "font-mono text-xs text-gray-600 text-right w-44",
                                    "{start_label} -> {end_label} | {duration_label}"
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[derive(Clone, PartialEq)]
struct RlPhaseSegment {
    name: String,
    phase: String,
    start: i64,
    end: i64,
    process_label: String,
    span_id: i64,
    attributes: Option<String>,
    turn_id: Option<String>,
    env_step_id: Option<String>,
}

#[derive(Clone, PartialEq)]
struct RlSampleRow {
    sample_key: String,
    label: String,
    detail: String,
    start: i64,
    end: i64,
    process_labels: String,
    segments: Vec<RlPhaseSegment>,
}

#[derive(Clone, PartialEq)]
struct RlRolloutView {
    rollout_id: String,
    start: i64,
    end: i64,
    samples: Vec<RlSampleRow>,
}

#[derive(Default)]
struct RlSampleBuilder {
    sample_key: String,
    label: String,
    rollout_id: String,
    group_id: Option<String>,
    attempt: Option<String>,
    start: i64,
    end: i64,
    process_labels: HashSet<String>,
    segments: Vec<RlPhaseSegment>,
}

#[component]
fn RolloutPerfettoTimeline(spans: Vec<DisplaySpan>, #[props(default)] fullscreen: bool) -> Element {
    let trace_json = build_chrome_trace_json_from_display_spans(&spans);
    let container_class = if fullscreen {
        "relative flex-1 min-h-0 overflow-hidden bg-white"
    } else {
        "relative min-h-[680px] overflow-hidden rounded border border-gray-200 bg-white"
    };
    let iframe_style = if fullscreen {
        "width: 100%; height: 100%; border: none; display: block;"
    } else {
        "width: 100%; height: 680px; border: none; display: block;"
    };

    rsx! {
        div {
            class: if fullscreen { "flex h-full min-h-0 flex-col" } else { "space-y-3" },
            if !fullscreen {
                div {
                    class: "rounded border border-gray-100 bg-gray-50 px-3 py-2 text-sm text-gray-600",
                    "Perfetto view for the currently loaded trace data. Use the Rollout ID filter above to focus this panel on one rollout."
                }
            }
            div {
                class: "{container_class}",
                if let Some(trace_json) = trace_json {
                    iframe {
                        srcdoc: tracing_viewer::get_tracing_viewer_html(&trace_json),
                        style: "{iframe_style}",
                        title: "Perfetto"
                    }
                } else {
                    div {
                        class: "absolute inset-0 flex items-center justify-center p-8",
                        div {
                            class: "text-center text-gray-500",
                            p { class: "mb-2 text-lg", "No timeline spans with end timestamps" }
                            p { class: "text-sm", "Load a rollout from the Rollout page, then return here." }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn RlTimeline(spans: Vec<DisplaySpan>) -> Element {
    let rollouts = build_rl_rollout_views(&spans);
    if rollouts.is_empty() {
        return rsx! {
            div {
                class: "text-sm text-gray-500",
                "{ROLLOUT_TIMELINE_EMPTY_HINT}"
            }
        };
    }

    let selected_rollout = use_signal(|| rollouts.first().map(|rollout| rollout.rollout_id.clone()).unwrap_or_default());
    let selected_sample = use_signal(|| String::new());
    let sort_mode = use_signal(|| "time".to_string());
    let selected_rollout_read = selected_rollout.read().clone();
    let selected_rollout_id = if rollouts.iter().any(|rollout| rollout.rollout_id == selected_rollout_read) {
        selected_rollout.read().clone()
    } else {
        rollouts.first().map(|rollout| rollout.rollout_id.clone()).unwrap_or_default()
    };
    let rollout = rollouts
        .iter()
        .find(|rollout| rollout.rollout_id == selected_rollout_id)
        .or_else(|| rollouts.first())
        .unwrap();
    let selected_sample_read = selected_sample.read().clone();
    let selected_sample_key = if rollout.samples.iter().any(|sample| sample.sample_key == selected_sample_read) {
        selected_sample.read().clone()
    } else {
        rollout.samples.first().map(|sample| sample.sample_key.clone()).unwrap_or_default()
    };
    let selected_sample_row = rollout.samples.iter().find(|sample| sample.sample_key == selected_sample_key);
    let min_start = rollout.start;
    let max_end = rollout.end.max(min_start + 1);
    let window = (max_end - min_start).max(1);
    let window_f = window as f64;
    let window_label = format_duration_ns(window);
    let sample_count = rollout.samples.len();
    let segment_count = rollout.samples.iter().map(|sample| sample.segments.len()).sum::<usize>();
    let sort_mode_value = sort_mode.read().clone();
    let mut display_samples = rollout.samples.clone();
    if sort_mode_value == "slow" {
        display_samples.sort_by(|a, b| sample_duration_ns(b).cmp(&sample_duration_ns(a))
            .then_with(|| a.start.cmp(&b.start)));
    } else {
        display_samples.sort_by_key(|sample| sample.start);
    }
    let slowest_sample = rollout.samples.iter().max_by_key(|sample| sample_duration_ns(sample));
    let slowest_label = slowest_sample
        .map(|sample| format!("slowest {} {}", sample.label, format_duration_ns(sample_duration_ns(sample))))
        .unwrap_or_default();
    let detail_bottom_pad = if selected_sample_row.is_some() {
        *RL_DETAIL_PANEL_HEIGHT.read() + 12
    } else {
        0
    };

    rsx! {
        div {
            class: "border-b border-gray-100",
            style: if detail_bottom_pad > 0 { format!("padding-bottom: {detail_bottom_pad}px") } else { String::new() },
            div {
                class: "mb-3 flex items-start justify-between gap-4",
                div {
                    h4 {
                        class: "text-sm font-semibold text-gray-900",
                        "Rollout"
                    }
                    div {
                        class: "text-xs text-gray-500",
                        "Grouped by rollout_id and trajectory_id. Each row is one rollout trajectory."
                    }
                }
                span {
                    class: "text-xs font-mono text-gray-500",
                    "window {window_label}"
                }
            }
            RlPhaseLegend {}
            div {
                class: "mb-3 flex flex-wrap items-center gap-3",
                span { class: "text-sm text-gray-600", "Rollout" }
                for rollout_option in rollouts.iter() {
                    {
                        let is_selected = rollout_option.rollout_id == selected_rollout_id;
                        let rollout_id = rollout_option.rollout_id.clone();
                        rsx! {
                            button {
                                class: format!("rounded px-2 py-1 text-xs font-mono {}", if is_selected { format!("bg-{} text-white", colors::PRIMARY) } else { "bg-gray-100 text-gray-700 hover:bg-gray-200".to_string() }),
                                onclick: {
                                    let mut selected_rollout = selected_rollout.clone();
                                    let mut selected_sample = selected_sample.clone();
                                    move |_| {
                                        *selected_rollout.write() = rollout_id.clone();
                                        *selected_sample.write() = String::new();
                                    }
                                },
                                "{rollout_option.rollout_id} ({rollout_option.samples.len()})"
                            }
                        }
                    }
                }
                span {
                    class: "ml-2 text-xs text-gray-500",
                "{sample_count} trajectories, {segment_count} phase spans"
                }
                if !slowest_label.is_empty() {
                    span {
                        class: "rounded bg-orange-50 px-2 py-1 text-xs font-mono text-orange-700",
                        "{slowest_label}"
                    }
                }
            }
            div {
                class: "mb-3 flex flex-wrap items-center gap-2 rounded border border-gray-100 bg-gray-50 px-3 py-2",
                span { class: "text-sm text-gray-600", "Sort" }
                button {
                    class: format!("rounded px-2 py-1 text-xs {}", if sort_mode_value == "time" { format!("bg-{} text-white", colors::PRIMARY) } else { "bg-white text-gray-700 hover:bg-gray-100".to_string() }),
                    onclick: {
                        let mut sort_mode = sort_mode.clone();
                        move |_| *sort_mode.write() = "time".to_string()
                    },
                    "timeline order"
                }
                button {
                    class: format!("rounded px-2 py-1 text-xs {}", if sort_mode_value == "slow" { format!("bg-{} text-white", colors::PRIMARY) } else { "bg-white text-gray-700 hover:bg-gray-100".to_string() }),
                    onclick: {
                        let mut sort_mode = sort_mode.clone();
                        move |_| *sort_mode.write() = "slow".to_string()
                    },
                    "slowest first"
                }
                span {
                    class: "text-xs text-gray-500",
                    "Tip: click a trajectory row to pin its phase details below."
                }
            }
            div {
                class: "space-y-1",
                for sample in display_samples.iter() {
                    {
                        let start_offset = sample.start - min_start;
                        let end_offset = sample.end - min_start;
                        let duration = (sample.end - sample.start).max(0);
                        let start_label = format_duration_ns(start_offset);
                        let end_label = format_duration_ns(end_offset);
                        let duration_label = format_duration_ns(duration);
                        let is_selected = sample.sample_key == selected_sample_key;
                        rsx! {
                            button {
                                class: format!("grid w-full grid-cols-[minmax(300px,34%)_1fr_auto] items-center gap-3 rounded px-2 py-1 text-left text-sm {}", if is_selected { "bg-blue-50 ring-1 ring-blue-200".to_string() } else { "hover:bg-gray-50".to_string() }),
                                onclick: {
                                    let mut selected_sample = selected_sample.clone();
                                    let sample_key = sample.sample_key.clone();
                                    move |_| *selected_sample.write() = sample_key.clone()
                                },
                                div {
                                    class: "min-w-0",
                                    div {
                                        class: "flex items-center gap-2 min-w-0",
                                        span {
                                            class: "font-medium text-gray-900 truncate",
                                            "{sample.label}"
                                        }
                                        if !sample.detail.is_empty() {
                                            span {
                                                class: "text-xs px-1.5 py-0.5 bg-gray-100 text-gray-600 rounded font-mono flex-shrink-0",
                                                "{sample.detail}"
                                            }
                                        }
                                        span {
                                            class: "text-xs px-1.5 py-0.5 bg-orange-50 text-orange-700 rounded font-mono flex-shrink-0",
                                            "{duration_label}"
                                        }
                                    }
                                    div {
                                        class: "mt-0.5 truncate text-xs text-gray-400",
                                        "{sample.process_labels}"
                                    }
                                }
                                div {
                                    class: "relative h-6 rounded bg-gray-100 overflow-hidden",
                                    for segment in sample.segments.iter() {
                                        {
                                            let left = ((segment.start - min_start) as f64 / window_f * 100.0).clamp(0.0, 100.0);
                                            let width = ((segment.end - segment.start).max(0) as f64 / window_f * 100.0)
                                                .max(0.35)
                                                .min((100.0 - left).max(0.35));
                                            let phase_class = rl_phase_bar_class(&segment.phase);
                                            let title = format!(
                                                "{} phase={} process={} duration={}",
                                                segment.name,
                                                segment.phase,
                                                segment.process_label,
                                                format_duration_ns((segment.end - segment.start).max(0)),
                                            );
                                            rsx! {
                                                div {
                                                    class: "{phase_class}",
                                                    title: "{title}",
                                                    style: "left: {left:.2}%; width: {width:.2}%;",
                                                }
                                            }
                                        }
                                    }
                                }
                                div {
                                    class: "font-mono text-xs text-gray-600 text-right w-44",
                                    "{start_label} -> {end_label} | {duration_label}"
                                }
                            }
                        }
                    }
                }
            }
            if let Some(sample) = selected_sample_row {
                RlSampleDetail { sample: sample.clone(), min_start }
            }
        }
    }
}

#[component]
fn TrainTimeline(spans: Vec<DisplaySpan>) -> Element {
    let rollouts = build_train_rollout_views(&spans);
    if rollouts.is_empty() {
        return rsx! {
            div {
                class: "text-sm text-gray-500",
                "{TRAIN_TIMELINE_EMPTY_HINT}"
            }
        };
    }

    let selected_rollout = use_signal(|| rollouts.first().map(|rollout| rollout.rollout_id.clone()).unwrap_or_default());
    let selected_batch = use_signal(|| String::new());
    let sort_mode = use_signal(|| "time".to_string());
    let selected_rollout_read = selected_rollout.read().clone();
    let selected_rollout_id = if rollouts.iter().any(|rollout| rollout.rollout_id == selected_rollout_read) {
        selected_rollout.read().clone()
    } else {
        rollouts.first().map(|rollout| rollout.rollout_id.clone()).unwrap_or_default()
    };
    let rollout = rollouts
        .iter()
        .find(|rollout| rollout.rollout_id == selected_rollout_id)
        .or_else(|| rollouts.first())
        .unwrap();
    let selected_batch_read = selected_batch.read().clone();
    let selected_batch_key = if rollout.samples.iter().any(|batch| batch.sample_key == selected_batch_read) {
        selected_batch.read().clone()
    } else {
        rollout.samples.first().map(|batch| batch.sample_key.clone()).unwrap_or_default()
    };
    let selected_batch_row = rollout.samples.iter().find(|batch| batch.sample_key == selected_batch_key);
    let min_start = rollout.start;
    let max_end = rollout.end.max(min_start + 1);
    let window = (max_end - min_start).max(1);
    let window_f = window as f64;
    let window_label = format_duration_ns(window);
    let batch_count = rollout.samples.len();
    let segment_count = rollout.samples.iter().map(|batch| batch.segments.len()).sum::<usize>();
    let sort_mode_value = sort_mode.read().clone();
    let mut display_batches = rollout.samples.clone();
    if sort_mode_value == "slow" {
        display_batches.sort_by(|a, b| sample_duration_ns(b).cmp(&sample_duration_ns(a))
            .then_with(|| a.start.cmp(&b.start)));
    } else {
        display_batches.sort_by_key(|batch| batch.start);
    }
    let slowest_batch = rollout.samples.iter().max_by_key(|batch| sample_duration_ns(batch));
    let slowest_label = slowest_batch
        .map(|batch| format!("slowest {} {}", batch.label, format_duration_ns(sample_duration_ns(batch))))
        .unwrap_or_default();
    let detail_bottom_pad = if selected_batch_row.is_some() {
        *RL_DETAIL_PANEL_HEIGHT.read() + 12
    } else {
        0
    };

    rsx! {
        div {
            class: "border-b border-gray-100",
            style: if detail_bottom_pad > 0 { format!("padding-bottom: {detail_bottom_pad}px") } else { String::new() },
            div {
                class: "mb-3 flex items-start justify-between gap-4",
                div {
                    h4 {
                        class: "text-sm font-semibold text-gray-900",
                        "Train"
                    }
                    div {
                        class: "text-xs text-gray-500",
                        "Grouped by rollout_id and train_step_id/batch_id. Each row is one train batch."
                    }
                }
                span {
                    class: "text-xs font-mono text-gray-500",
                    "window {window_label}"
                }
            }
            RlPhaseLegend {}
            div {
                class: "mb-3 flex flex-wrap items-center gap-3",
                span { class: "text-sm text-gray-600", "Rollout" }
                for rollout_option in rollouts.iter() {
                    {
                        let is_selected = rollout_option.rollout_id == selected_rollout_id;
                        let rollout_id = rollout_option.rollout_id.clone();
                        rsx! {
                            button {
                                class: format!("rounded px-2 py-1 text-xs font-mono {}", if is_selected { format!("bg-{} text-white", colors::PRIMARY) } else { "bg-gray-100 text-gray-700 hover:bg-gray-200".to_string() }),
                                onclick: {
                                    let mut selected_rollout = selected_rollout.clone();
                                    let mut selected_batch = selected_batch.clone();
                                    move |_| {
                                        *selected_rollout.write() = rollout_id.clone();
                                        *selected_batch.write() = String::new();
                                    }
                                },
                                "{rollout_option.rollout_id} ({rollout_option.samples.len()})"
                            }
                        }
                    }
                }
                span {
                    class: "ml-2 text-xs text-gray-500",
                    "{batch_count} batches, {segment_count} phase spans"
                }
                if !slowest_label.is_empty() {
                    span {
                        class: "rounded bg-orange-50 px-2 py-1 text-xs font-mono text-orange-700",
                        "{slowest_label}"
                    }
                }
            }
            div {
                class: "mb-3 flex flex-wrap items-center gap-2 rounded border border-gray-100 bg-gray-50 px-3 py-2",
                span { class: "text-sm text-gray-600", "Sort" }
                button {
                    class: format!("rounded px-2 py-1 text-xs {}", if sort_mode_value == "time" { format!("bg-{} text-white", colors::PRIMARY) } else { "bg-white text-gray-700 hover:bg-gray-100".to_string() }),
                    onclick: {
                        let mut sort_mode = sort_mode.clone();
                        move |_| *sort_mode.write() = "time".to_string()
                    },
                    "timeline order"
                }
                button {
                    class: format!("rounded px-2 py-1 text-xs {}", if sort_mode_value == "slow" { format!("bg-{} text-white", colors::PRIMARY) } else { "bg-white text-gray-700 hover:bg-gray-100".to_string() }),
                    onclick: {
                        let mut sort_mode = sort_mode.clone();
                        move |_| *sort_mode.write() = "slow".to_string()
                    },
                    "slowest first"
                }
                span {
                    class: "text-xs text-gray-500",
                    "Tip: click a train batch row to pin its phase details below."
                }
            }
            div {
                class: "space-y-1",
                for batch in display_batches.iter() {
                    {
                        let start_offset = batch.start - min_start;
                        let end_offset = batch.end - min_start;
                        let duration = (batch.end - batch.start).max(0);
                        let start_label = format_duration_ns(start_offset);
                        let end_label = format_duration_ns(end_offset);
                        let duration_label = format_duration_ns(duration);
                        let is_selected = batch.sample_key == selected_batch_key;
                        rsx! {
                            button {
                                class: format!("grid w-full grid-cols-[minmax(300px,34%)_1fr_auto] items-center gap-3 rounded px-2 py-1 text-left text-sm {}", if is_selected { "bg-blue-50 ring-1 ring-blue-200".to_string() } else { "hover:bg-gray-50".to_string() }),
                                onclick: {
                                    let mut selected_batch = selected_batch.clone();
                                    let batch_key = batch.sample_key.clone();
                                    move |_| *selected_batch.write() = batch_key.clone()
                                },
                                div {
                                    class: "min-w-0",
                                    div {
                                        class: "flex items-center gap-2 min-w-0",
                                        span {
                                            class: "font-medium text-gray-900 truncate",
                                            "{batch.label}"
                                        }
                                        if !batch.detail.is_empty() {
                                            span {
                                                class: "text-xs px-1.5 py-0.5 bg-gray-100 text-gray-600 rounded font-mono flex-shrink-0",
                                                "{batch.detail}"
                                            }
                                        }
                                        span {
                                            class: "text-xs px-1.5 py-0.5 bg-orange-50 text-orange-700 rounded font-mono flex-shrink-0",
                                            "{duration_label}"
                                        }
                                    }
                                    div {
                                        class: "mt-0.5 truncate text-xs text-gray-400",
                                        "{batch.process_labels}"
                                    }
                                }
                                div {
                                    class: "relative h-6 rounded bg-gray-100 overflow-hidden",
                                    for segment in batch.segments.iter() {
                                        {
                                            let left = ((segment.start - min_start) as f64 / window_f * 100.0).clamp(0.0, 100.0);
                                            let width = ((segment.end - segment.start).max(0) as f64 / window_f * 100.0)
                                                .max(0.35)
                                                .min((100.0 - left).max(0.35));
                                            let phase_class = rl_phase_bar_class(&segment.phase);
                                            let title = format!(
                                                "{} phase={} process={} duration={}",
                                                segment.name,
                                                segment.phase,
                                                segment.process_label,
                                                format_duration_ns((segment.end - segment.start).max(0)),
                                            );
                                            rsx! {
                                                div {
                                                    class: "{phase_class}",
                                                    title: "{title}",
                                                    style: "left: {left:.2}%; width: {width:.2}%;",
                                                }
                                            }
                                        }
                                    }
                                }
                                div {
                                    class: "font-mono text-xs text-gray-600 text-right w-44",
                                    "{start_label} -> {end_label} | {duration_label}"
                                }
                            }
                        }
                    }
                }
            }
            if let Some(batch) = selected_batch_row {
                TrainBatchDetail { batch: batch.clone(), min_start }
            }
        }
    }
}

#[component]
fn ResizableDetailPanel(
    title: String,
    subtitle: String,
    segment_count: usize,
    children: Element,
) -> Element {
    let mut panel_height = use_signal(|| *RL_DETAIL_PANEL_HEIGHT.read());
    let mut is_resizing = use_signal(|| false);
    let mut drag_start_y = use_signal(|| 0.0f64);
    let mut drag_start_height = use_signal(|| 0.0f64);
    let height_px = *panel_height.read();
    let height_label = format!("{height_px}px");
    let resizing = *is_resizing.read();

    rsx! {
        if resizing {
            div {
                class: "fixed inset-0 z-[60] cursor-ns-resize select-none",
                style: "touch-action: none;",
                onmousemove: {
                    let mut panel_height = panel_height.clone();
                    move |ev| {
                        let current_y = ev.client_coordinates().y;
                        let delta = *drag_start_y.read() - current_y;
                        let new_height = (*drag_start_height.read() + delta)
                            .max(RL_DETAIL_PANEL_HEIGHT_MIN as f64)
                            .min(RL_DETAIL_PANEL_HEIGHT_MAX as f64) as u32;
                        *panel_height.write() = new_height;
                        *RL_DETAIL_PANEL_HEIGHT.write() = new_height;
                    }
                },
                onmouseup: {
                    let mut is_resizing = is_resizing.clone();
                    move |_| {
                        *is_resizing.write() = false;
                    }
                },
            }
        }
        div {
            class: "fixed bottom-0 left-0 right-0 z-50 flex flex-col border-t border-gray-200 bg-white/95 shadow-2xl backdrop-blur",
            style: "height: {height_px}px;",
            div {
                class: "group flex h-3 shrink-0 cursor-ns-resize items-center justify-center border-b border-gray-100 bg-gray-50 hover:bg-gray-100 active:bg-gray-200",
                title: "Drag up/down to resize panel height",
                onmousedown: {
                    let mut is_resizing = is_resizing.clone();
                    let mut drag_start_y = drag_start_y.clone();
                    let mut drag_start_height = drag_start_height.clone();
                    move |ev| {
                        *is_resizing.write() = true;
                        *drag_start_y.write() = ev.client_coordinates().y;
                        *drag_start_height.write() = *panel_height.read() as f64;
                        ev.prevent_default();
                    }
                },
                div { class: "h-1 w-16 rounded-full bg-gray-300 group-hover:bg-gray-400 group-active:bg-gray-500" }
            }
            div {
                class: "mx-auto flex min-h-0 w-full max-w-[1400px] flex-1 flex-col px-3 pb-3",
                div {
                    class: "mb-2 flex shrink-0 items-center justify-between gap-3 pt-2",
                    div {
                        class: "min-w-0",
                        h5 { class: "text-sm font-semibold text-gray-900", "{title}" }
                        div { class: "text-xs text-gray-500 truncate", "{subtitle}" }
                    }
                    div {
                        class: "flex shrink-0 items-center gap-2",
                        span {
                            class: "hidden sm:inline text-xs font-mono text-gray-500",
                            "{segment_count} phases · {height_label}"
                        }
                        button {
                            class: "rounded border border-gray-200 bg-white px-2 py-0.5 text-xs text-gray-700 hover:bg-gray-50",
                            title: "Expand to fit all phases",
                            onclick: {
                                let mut panel_height = panel_height.clone();
                                move |_| {
                                    let h = estimate_detail_panel_height(segment_count);
                                    *panel_height.write() = h;
                                    *RL_DETAIL_PANEL_HEIGHT.write() = h;
                                }
                            },
                            "Expand all"
                        }
                        button {
                            class: "rounded border border-gray-200 bg-white px-2 py-0.5 text-xs text-gray-700 hover:bg-gray-50",
                            title: "Reset panel height",
                            onclick: {
                                let mut panel_height = panel_height.clone();
                                move |_| {
                                    *panel_height.write() = RL_DETAIL_PANEL_HEIGHT_DEFAULT;
                                    *RL_DETAIL_PANEL_HEIGHT.write() = RL_DETAIL_PANEL_HEIGHT_DEFAULT;
                                }
                            },
                            "Reset"
                        }
                    }
                }
                div {
                    class: "min-h-0 flex-1 space-y-1 overflow-y-auto pr-1",
                    {children}
                }
            }
        }
    }
}

#[component]
fn TrainBatchDetail(batch: RlSampleRow, min_start: i64) -> Element {
    let batch_duration = format_duration_ns(sample_duration_ns(&batch));
    rsx! {
        ResizableDetailPanel {
            title: format!("Train Batch Detail: {}", batch.label),
            subtitle: format!("{} | duration {}", batch.detail, batch_duration),
            segment_count: batch.segments.len(),
            for segment in batch.segments.iter() {
                {
                    let start = format_duration_ns(segment.start - min_start);
                    let end = format_duration_ns(segment.end - min_start);
                    let duration = format_duration_ns((segment.end - segment.start).max(0));
                    let dot_class = rl_phase_dot_class(&segment.phase);
                    rsx! {
                        div {
                            class: "grid grid-cols-[minmax(180px,22%)_130px_1fr] items-start gap-2 rounded bg-gray-50 px-2 py-1 text-xs",
                            div {
                                class: "min-w-0",
                                div {
                                    class: "flex items-center gap-2",
                                    span { class: "{dot_class}" }
                                    span { class: "font-medium text-gray-900 truncate", "{segment.name}" }
                                }
                                div { class: "font-mono text-gray-400", "span:{segment.span_id}" }
                            }
                            div { class: "font-mono text-gray-600", "{segment.phase}" }
                            div {
                                class: "min-w-0",
                                div { class: "font-mono text-gray-600", "{start} -> {end} | {duration}" }
                                div { class: "truncate text-gray-400", "{segment.process_label}" }
                                if let Some(ref attrs) = segment.attributes {
                                    if !attrs.is_empty() {
                                        div { class: "mt-0.5 truncate font-mono text-gray-400", "{attrs}" }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn RlPhaseLegend() -> Element {
    rsx! {
        div {
            class: "mb-3 flex flex-wrap items-center gap-3 rounded border border-gray-100 bg-gray-50 px-3 py-2 text-xs text-gray-600",
            span { class: "font-medium text-gray-700", "Phase legend" }
            RlLegendItem { class_name: "bg-blue-500".to_string(), label: "inference".to_string() }
            RlLegendItem { class_name: "bg-amber-500".to_string(), label: "env step".to_string() }
            RlLegendItem { class_name: "bg-violet-500".to_string(), label: "tool call".to_string() }
            RlLegendItem { class_name: "bg-pink-500".to_string(), label: "reward".to_string() }
            RlLegendItem { class_name: "bg-emerald-500".to_string(), label: "train/optimizer".to_string() }
            RlLegendItem { class_name: "bg-slate-500".to_string(), label: "other".to_string() }
        }
    }
}

#[component]
fn RlLegendItem(class_name: String, label: String) -> Element {
    rsx! {
        span {
            class: "inline-flex items-center gap-1.5",
            span { class: "h-2.5 w-2.5 rounded {class_name}" }
            "{label}"
        }
    }
}

#[component]
fn RlSampleDetail(sample: RlSampleRow, min_start: i64) -> Element {
    let sample_duration = format_duration_ns(sample_duration_ns(&sample));
    rsx! {
        ResizableDetailPanel {
            title: format!("Trajectory Detail: {}", sample.label),
            subtitle: format!("{} | duration {}", sample.detail, sample_duration),
            segment_count: sample.segments.len(),
            for segment in sample.segments.iter() {
                {
                    let start = format_duration_ns(segment.start - min_start);
                    let end = format_duration_ns(segment.end - min_start);
                    let duration = format_duration_ns((segment.end - segment.start).max(0));
                    let turn = segment.turn_id.clone().unwrap_or_else(|| "-".to_string());
                    let env_step = segment.env_step_id.clone().unwrap_or_else(|| "-".to_string());
                    let dot_class = rl_phase_dot_class(&segment.phase);
                    rsx! {
                        div {
                            class: "grid grid-cols-[minmax(180px,22%)_130px_90px_90px_1fr] items-start gap-2 rounded bg-gray-50 px-2 py-1 text-xs",
                            div {
                                class: "min-w-0",
                                div {
                                    class: "flex items-center gap-2",
                                    span { class: "{dot_class}" }
                                    span { class: "font-medium text-gray-900 truncate", "{segment.name}" }
                                }
                                div { class: "font-mono text-gray-400", "span:{segment.span_id}" }
                            }
                            div { class: "font-mono text-gray-600", "{segment.phase}" }
                            div { class: "font-mono text-gray-500", "turn:{turn}" }
                            div { class: "font-mono text-gray-500", "env:{env_step}" }
                            div {
                                class: "min-w-0",
                                div { class: "font-mono text-gray-600", "{start} -> {end} | {duration}" }
                                div { class: "truncate text-gray-400", "{segment.process_label}" }
                                if let Some(ref attrs) = segment.attributes {
                                    if !attrs.is_empty() {
                                        div { class: "mt-0.5 truncate font-mono text-gray-400", "{attrs}" }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn build_timeline_spans(spans: &[DisplaySpan], max_depth: usize, selected_batch_key: &str) -> Vec<TimelineSpan> {
    let mut rows = Vec::new();
    if !selected_batch_key.is_empty() {
        if let Some(selected) = find_display_span(spans, selected_batch_key) {
            collect_timeline_spans(std::slice::from_ref(selected), 0, max_depth, &mut rows);
        }
    } else {
        collect_timeline_spans(spans, 0, max_depth, &mut rows);
    }
    rows.sort_by_key(|row| row.start);
    rows
}

fn build_rl_rollout_views(spans: &[DisplaySpan]) -> Vec<RlRolloutView> {
    let mut all_spans = Vec::<DisplaySpan>::new();
    collect_all_display_spans(spans, &mut all_spans);

    let mut samples = HashMap::<String, RlSampleBuilder>::new();

    for span in all_spans {
        let Some(segment) = rl_segment_from_span(&span) else {
            continue;
        };
        let Some(rollout_id) = attr_string(&span.attributes, "rollout_id")
            .or_else(|| attr_string(&span.attributes, "step_id")) else {
            continue;
        };
        let Some(trajectory_id) = attr_string(&span.attributes, "trajectory_id")
            .or_else(|| {
                let group_id = attr_string(&span.attributes, "group_id")?;
                let group_index = attr_string(&span.attributes, "group_index")?;
                Some(format!("r{rollout_id}-g{group_id}-i{group_index}"))
            })
            .or_else(|| attr_string(&span.attributes, "sample_id")) else {
            continue;
        };
        let sample_key = format!("{rollout_id}:{trajectory_id}");
        let entry = samples.entry(sample_key.clone()).or_insert_with(|| RlSampleBuilder {
            sample_key: sample_key.clone(),
            label: trajectory_id.clone(),
            rollout_id: rollout_id.clone(),
            group_id: attr_string(&span.attributes, "group_id"),
            attempt: attr_string(&span.attributes, "attempt"),
            start: segment.start,
            end: segment.end,
            ..Default::default()
        });

        entry.start = entry.start.min(segment.start);
        entry.end = entry.end.max(segment.end);
        if entry.group_id.is_none() {
            entry.group_id = attr_string(&span.attributes, "group_id");
        }
        if entry.attempt.is_none() {
            entry.attempt = attr_string(&span.attributes, "attempt");
        }
        entry.process_labels.insert(span.process_label.clone());
        entry.segments.push(segment);
    }

    let mut by_rollout = HashMap::<String, Vec<RlSampleRow>>::new();
    for mut builder in samples.into_values() {
        builder.segments.sort_by_key(|segment| segment.start);
        let mut labels = builder.process_labels.into_iter().collect::<Vec<_>>();
        labels.sort();
        let mut detail_parts = Vec::new();
        if let Some(group_id) = builder.group_id.filter(|value| !value.is_empty()) {
            detail_parts.push(format!("group:{group_id}"));
        }
        if let Some(attempt) = builder.attempt.filter(|value| !value.is_empty()) {
            detail_parts.push(format!("attempt:{attempt}"));
        }
        detail_parts.push(format!("{} phases", builder.segments.len()));
        by_rollout.entry(builder.rollout_id.clone()).or_default().push(RlSampleRow {
            sample_key: builder.sample_key,
            label: builder.label,
            detail: detail_parts.join(" "),
            start: builder.start,
            end: builder.end,
            process_labels: labels.join(", "),
            segments: builder.segments,
        });
    }

    let mut rollouts = by_rollout
        .into_iter()
        .map(|(rollout_id, mut samples)| {
            samples.sort_by_key(|sample| sample.start);
            let start = samples.iter().map(|sample| sample.start).min().unwrap_or(0);
            let end = samples.iter().map(|sample| sample.end).max().unwrap_or(start);
            RlRolloutView {
                rollout_id,
                start,
                end,
                samples,
            }
        })
        .collect::<Vec<_>>();
    rollouts.sort_by(|a, b| {
        numeric_string_sort_key(&a.rollout_id)
            .cmp(&numeric_string_sort_key(&b.rollout_id))
            .then_with(|| a.rollout_id.cmp(&b.rollout_id))
    });
    rollouts
}

fn build_train_rollout_views(spans: &[DisplaySpan]) -> Vec<RlRolloutView> {
    let mut all_spans = Vec::<DisplaySpan>::new();
    collect_all_display_spans(spans, &mut all_spans);

    let mut batches = HashMap::<String, RlSampleBuilder>::new();

    for span in all_spans {
        let Some(segment) = train_segment_from_span(&span) else {
            continue;
        };
        let Some(rollout_id) = attr_string(&span.attributes, "rollout_id")
            .or_else(|| attr_string(&span.attributes, "step_id")) else {
            continue;
        };
        let train_step_id = attr_string(&span.attributes, "train_step_id");
        let batch_id = attr_string(&span.attributes, "batch_id");
        let Some(batch_key_id) = train_step_id.clone().or_else(|| batch_id.clone()) else {
            continue;
        };

        let batch_key = format!("{rollout_id}:{batch_key_id}");
        let label = match (train_step_id, batch_id) {
            (Some(train_step_id), Some(batch_id)) => format!("train_step:{train_step_id} batch:{batch_id}"),
            (Some(train_step_id), None) => format!("train_step:{train_step_id}"),
            (None, Some(batch_id)) => format!("batch:{batch_id}"),
            (None, None) => batch_key_id,
        };
        let entry = batches.entry(batch_key.clone()).or_insert_with(|| RlSampleBuilder {
            sample_key: batch_key.clone(),
            label,
            rollout_id: rollout_id.clone(),
            group_id: attr_string(&span.attributes, "role"),
            attempt: attr_string(&span.attributes, "rank"),
            start: segment.start,
            end: segment.end,
            ..Default::default()
        });

        entry.start = entry.start.min(segment.start);
        entry.end = entry.end.max(segment.end);
        if entry.group_id.is_none() {
            entry.group_id = attr_string(&span.attributes, "role");
        }
        if entry.attempt.is_none() {
            entry.attempt = attr_string(&span.attributes, "rank");
        }
        entry.process_labels.insert(span.process_label.clone());
        entry.segments.push(segment);
    }

    let mut by_rollout = HashMap::<String, Vec<RlSampleRow>>::new();
    for mut builder in batches.into_values() {
        builder.segments.sort_by_key(|segment| segment.start);
        let mut labels = builder.process_labels.into_iter().collect::<Vec<_>>();
        labels.sort();
        let mut detail_parts = Vec::new();
        if let Some(role) = builder.group_id.filter(|value| !value.is_empty()) {
            detail_parts.push(format!("role:{role}"));
        }
        if let Some(rank) = builder.attempt.filter(|value| !value.is_empty()) {
            detail_parts.push(format!("rank:{rank}"));
        }
        detail_parts.push(format!("{} phases", builder.segments.len()));
        by_rollout.entry(builder.rollout_id.clone()).or_default().push(RlSampleRow {
            sample_key: builder.sample_key,
            label: builder.label,
            detail: detail_parts.join(" "),
            start: builder.start,
            end: builder.end,
            process_labels: labels.join(", "),
            segments: builder.segments,
        });
    }

    let mut rollouts = by_rollout
        .into_iter()
        .map(|(rollout_id, mut samples)| {
            samples.sort_by_key(|sample| sample.start);
            let start = samples.iter().map(|sample| sample.start).min().unwrap_or(0);
            let end = samples.iter().map(|sample| sample.end).max().unwrap_or(start);
            RlRolloutView {
                rollout_id,
                start,
                end,
                samples,
            }
        })
        .collect::<Vec<_>>();
    rollouts.sort_by(|a, b| {
        numeric_string_sort_key(&a.rollout_id)
            .cmp(&numeric_string_sort_key(&b.rollout_id))
            .then_with(|| a.rollout_id.cmp(&b.rollout_id))
    });
    rollouts
}

fn collect_all_display_spans(spans: &[DisplaySpan], out: &mut Vec<DisplaySpan>) {
    for span in spans {
        out.push(span.clone());
        collect_all_display_spans(&span.children, out);
    }
}

fn build_chrome_trace_json_from_display_spans(spans: &[DisplaySpan]) -> Option<String> {
    let mut all_spans = Vec::<DisplaySpan>::new();
    collect_all_display_spans(spans, &mut all_spans);
    let mut complete_spans = all_spans
        .into_iter()
        .filter(|span| span.end_timestamp.is_some())
        .collect::<Vec<_>>();
    if complete_spans.is_empty() {
        return None;
    }
    complete_spans.sort_by(|a, b| {
        a.start_timestamp
            .cmp(&b.start_timestamp)
            .then_with(|| {
                let a_duration = a.end_timestamp.unwrap_or(a.start_timestamp) - a.start_timestamp;
                let b_duration = b.end_timestamp.unwrap_or(b.start_timestamp) - b.start_timestamp;
                b_duration.cmp(&a_duration)
            })
    });

    let min_start = complete_spans
        .iter()
        .map(|span| span.start_timestamp)
        .min()
        .unwrap_or(0);

    let mut process_labels = complete_spans
        .iter()
        .map(perfetto_process_label)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    process_labels.sort();
    let process_ids = process_labels
        .iter()
        .enumerate()
        .map(|(idx, label)| (label.clone(), (idx + 1) as i64))
        .collect::<HashMap<_, _>>();

    let track_keys = complete_spans
        .iter()
        .map(|span| (perfetto_process_label(span), perfetto_track_label(span)))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let mut track_keys = track_keys;
    track_keys.sort();
    let track_ids = track_keys
        .iter()
        .enumerate()
        .map(|(idx, key)| (key.clone(), (idx + 1) as i64))
        .collect::<HashMap<_, _>>();

    let mut trace_events = Vec::<serde_json::Value>::new();
    for label in process_labels {
        let pid = *process_ids.get(&label).unwrap_or(&1);
        trace_events.push(serde_json::json!({
            "name": "process_name",
            "ph": "M",
            "pid": pid,
            "tid": 0,
            "args": {
                "name": label,
            },
        }));
    }
    for (process_label, track_label) in track_keys {
        let pid = *process_ids.get(&process_label).unwrap_or(&1);
        let tid = *track_ids.get(&(process_label.clone(), track_label.clone())).unwrap_or(&1);
        trace_events.push(serde_json::json!({
            "name": "thread_name",
            "ph": "M",
            "pid": pid,
            "tid": tid,
            "args": {
                "name": track_label,
            },
        }));
    }

    for span in complete_spans {
        let end = span.end_timestamp.unwrap_or(span.start_timestamp);
        let duration = (end - span.start_timestamp).max(0) / 1000;
        let ts = (span.start_timestamp - min_start).max(0) / 1000;
        let process_label = perfetto_process_label(&span);
        let pid = *process_ids.get(&process_label).unwrap_or(&1);
        let track_label = perfetto_track_label(&span);
        let tid = *track_ids
            .get(&(process_label.clone(), track_label.clone()))
            .unwrap_or(&span.thread_id);
        let mut args = serde_json::Map::new();
        args.insert("span_id".to_string(), serde_json::json!(span.span_id));
        args.insert("trace_id".to_string(), serde_json::json!(span.trace_id));
        args.insert("process".to_string(), serde_json::json!(span.process_label));
        args.insert("perfetto_process".to_string(), serde_json::json!(process_label));
        args.insert("track".to_string(), serde_json::json!(track_label));
        if let Some(parent_id) = span.parent_id {
            args.insert("parent_id".to_string(), serde_json::json!(parent_id));
        }
        if let Some(location) = span.location.as_ref().filter(|value| !value.is_empty()) {
            args.insert("location".to_string(), serde_json::json!(location));
        }
        if let Some(attrs) = span.attributes.as_ref().filter(|value| !value.is_empty()) {
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(attrs) {
                args.insert("attributes".to_string(), parsed);
            }
        }

        trace_events.push(serde_json::json!({
            "name": span.name,
            "cat": span.kind.unwrap_or_else(|| "span".to_string()),
            "ph": "X",
            "ts": ts,
            "dur": duration,
            "pid": pid,
            "tid": tid,
            "args": args,
        }));
    }

    Some(serde_json::json!({
        "traceEvents": trace_events,
        "displayTimeUnit": "ms",
    }).to_string())
}

fn perfetto_process_label(span: &DisplaySpan) -> String {
    if let Some(rollout_id) = attr_string(&span.attributes, "rollout_id") {
        return format!("rollout:{rollout_id}");
    }
    span.process_label.clone()
}

fn perfetto_track_label(span: &DisplaySpan) -> String {
    let rollout_id = attr_string(&span.attributes, "rollout_id");
    let trajectory_id = attr_string(&span.attributes, "trajectory_id")
        .or_else(|| attr_string(&span.attributes, "sample_id"))
        .or_else(|| {
            let group_id = attr_string(&span.attributes, "group_id")?;
            let group_index = attr_string(&span.attributes, "group_index")?;
            Some(format!("group:{group_id}/sample:{group_index}"))
        });

    if let Some(rollout_id) = rollout_id {
        if let Some(trajectory_id) = trajectory_id {
            return format!("rollout:{rollout_id} / {trajectory_id}");
        }
        if span.name.starts_with("rollout.") || span.kind.as_deref().unwrap_or("").contains("rollout") {
            return format!("rollout:{rollout_id} / control");
        }
        return format!("rollout:{rollout_id}");
    }

    if let Some(batch_id) = attr_string(&span.attributes, "batch_id")
        .or_else(|| attr_string(&span.attributes, "train_step_id"))
    {
        return format!("train:{batch_id}");
    }

    format!("{} / thread:{}", span.process_label, span.thread_id)
}

fn rl_segment_from_span(span: &DisplaySpan) -> Option<RlPhaseSegment> {
    let end = span.end_timestamp?;
    if attr_string(&span.attributes, "sample_id").is_none()
        && attr_string(&span.attributes, "trajectory_id").is_none()
    {
        return None;
    }
    let phase = attr_string(&span.attributes, "phase")
        .or_else(|| span.kind.clone())
        .unwrap_or_else(|| span.name.clone());

    Some(RlPhaseSegment {
        name: span.name.clone(),
        phase,
        start: span.start_timestamp,
        end,
        process_label: span.process_label.clone(),
        span_id: span.span_id,
        attributes: span.attributes.clone(),
        turn_id: attr_string(&span.attributes, "turn_id"),
        env_step_id: attr_string(&span.attributes, "env_step_id"),
    })
}

fn train_segment_from_span(span: &DisplaySpan) -> Option<RlPhaseSegment> {
    let end = span.end_timestamp?;
    let phase = attr_string(&span.attributes, "phase")
        .or_else(|| span.kind.clone())
        .unwrap_or_else(|| span.name.clone());
    if !is_train_timeline_span(
        &span.name,
        &phase,
        span.kind.as_deref(),
        &span.attributes,
    ) {
        return None;
    }

    Some(RlPhaseSegment {
        name: span.name.clone(),
        phase,
        start: span.start_timestamp,
        end,
        process_label: span.process_label.clone(),
        span_id: span.span_id,
        attributes: span.attributes.clone(),
        turn_id: None,
        env_step_id: None,
    })
}

fn numeric_string_sort_key(value: &str) -> i64 {
    value.parse::<i64>().unwrap_or(i64::MAX)
}

fn sample_duration_ns(sample: &RlSampleRow) -> i64 {
    (sample.end - sample.start).max(0)
}

fn rl_phase_bar_class(phase: &str) -> &'static str {
    if phase.contains("inference") || phase.contains("generate") {
        "absolute top-1 bottom-1 rounded bg-blue-500"
    } else if phase.contains("env") {
        "absolute top-1 bottom-1 rounded bg-amber-500"
    } else if phase.contains("tool") {
        "absolute top-1 bottom-1 rounded bg-violet-500"
    } else if phase.contains("reward") {
        "absolute top-1 bottom-1 rounded bg-pink-500"
    } else if phase.contains("train") || phase.contains("optimizer") {
        "absolute top-1 bottom-1 rounded bg-emerald-500"
    } else {
        "absolute top-1 bottom-1 rounded bg-slate-500"
    }
}

fn rl_phase_dot_class(phase: &str) -> &'static str {
    if phase.contains("inference") || phase.contains("generate") {
        "h-2.5 w-2.5 flex-shrink-0 rounded-full bg-blue-500"
    } else if phase.contains("env") {
        "h-2.5 w-2.5 flex-shrink-0 rounded-full bg-amber-500"
    } else if phase.contains("tool") {
        "h-2.5 w-2.5 flex-shrink-0 rounded-full bg-violet-500"
    } else if phase.contains("reward") {
        "h-2.5 w-2.5 flex-shrink-0 rounded-full bg-pink-500"
    } else if phase.contains("train") || phase.contains("optimizer") {
        "h-2.5 w-2.5 flex-shrink-0 rounded-full bg-emerald-500"
    } else {
        "h-2.5 w-2.5 flex-shrink-0 rounded-full bg-slate-500"
    }
}

fn collect_timeline_spans(spans: &[DisplaySpan], depth: usize, max_depth: usize, rows: &mut Vec<TimelineSpan>) {
    for span in spans {
        if depth < max_depth {
            if let Some(end) = span.end_timestamp {
                rows.push(TimelineSpan {
                    unique_key: span.unique_key.clone(),
                    name: span.name.clone(),
                    display_id: span.display_id.clone(),
                    span_id: span.span_id,
                    kind: span.kind.clone(),
                    location: span.location.clone(),
                    start: span.start_timestamp,
                    duration: (end - span.start_timestamp).max(0),
                    depth,
                    process_label: span.process_label.clone(),
                });
            }
        }
        collect_timeline_spans(&span.children, depth + 1, max_depth, rows);
    }
}

fn build_display_spans(spans: &[SpanInfo]) -> Vec<DisplaySpan> {
    let mut counters = HashMap::<String, usize>::new();
    let mut sorted = spans.to_vec();
    sorted.sort_by_key(|span| span.start_timestamp);
    sorted
        .iter()
        .map(|span| build_display_span(span, &mut counters))
        .collect()
}

fn build_display_span(span: &SpanInfo, counters: &mut HashMap<String, usize>) -> DisplaySpan {
    let counter_key = span.phase
        .as_ref()
        .map(|phase| format!("{}:{phase}", span.name))
        .unwrap_or_else(|| span.name.clone());
    let next_id = counters.entry(counter_key).or_insert(0);
    *next_id += 1;
    let display_id = format!("{}#{}", span.name, *next_id);
    let process_label = process_label(&span.attributes, span.trace_id);
    let unique_key = format!("{}:{}:{}", process_label, span.trace_id, span.span_id);

    let mut children = span.children.clone();
    children.sort_by_key(|child| child.start_timestamp);
    let children = children
        .iter()
        .map(|child| build_display_span(child, counters))
        .collect();

    DisplaySpan {
        unique_key,
        span_id: span.span_id,
        trace_id: span.trace_id,
        parent_id: span.parent_id,
        name: span.name.clone(),
        display_id,
        start_timestamp: span.start_timestamp,
        end_timestamp: span.end_timestamp,
        thread_id: span.thread_id,
        kind: span.phase.clone(),
        location: span.location.clone(),
        attributes: span.attributes.clone(),
        process_label,
        children,
        events: span.events.clone(),
    }
}

fn collect_process_options(spans: &[DisplaySpan], discovered_processes: &[TraceProcessInfo]) -> Vec<FilterOption> {
    let mut by_pid = HashMap::<String, FilterOption>::new();
    let mut all_spans = Vec::<DisplaySpan>::new();
    collect_all_display_spans(spans, &mut all_spans);

    for process in discovered_processes {
        by_pid.entry(process.pid.to_string()).or_insert(FilterOption {
            filter_key: format!("pid:{}", process.pid),
            label: trace_process_label(process),
        });
    }

    for span in all_spans {
        let Some(pid) = process_pid_from_attrs(&span.attributes) else {
            continue;
        };
        let role = attr_string(&span.attributes, "process_role").unwrap_or_else(|| "process".to_string());
        let actor_name = attr_string(&span.attributes, "ray_actor_name");
        let worker_id = attr_string(&span.attributes, "ray_worker_id")
            .map(|id| short_id(&id));
        let mut parts = vec![format!("{role}:{pid}")];
        if let Some(actor_name) = actor_name.filter(|value| !value.is_empty()) {
            parts.push(actor_name);
        }
        if let Some(worker_id) = worker_id.filter(|value| !value.is_empty()) {
            parts.push(format!("worker:{worker_id}"));
        }
        by_pid.entry(pid.clone()).or_insert(FilterOption {
            filter_key: format!("pid:{pid}"),
            label: parts.join(" | "),
        });
    }

    let mut options = by_pid.into_values().collect::<Vec<_>>();
    options.sort_by(|a, b| process_option_rank(a).cmp(&process_option_rank(b))
        .then_with(|| a.label.cmp(&b.label)));
    options
}

fn trace_process_label(process: &TraceProcessInfo) -> String {
    let role = process.process_role.as_deref().unwrap_or("process");
    let mut parts = vec![format!("{role}:{}", process.pid)];
    if let Some(actor_name) = process.ray_actor_name.as_ref().filter(|value| !value.is_empty()) {
        parts.push(actor_name.clone());
    }
    if let Some(worker_id) = process.ray_worker_id.as_ref().filter(|value| !value.is_empty()) {
        parts.push(format!("worker:{}", short_id(worker_id)));
    }
    parts.join(" | ")
}

fn collect_function_options(spans: &[DisplaySpan]) -> Vec<FilterOption> {
    let mut counts = HashMap::<String, usize>::new();
    let mut all_spans = Vec::<DisplaySpan>::new();
    collect_all_display_spans(spans, &mut all_spans);

    for span in all_spans {
        *counts.entry(span.name).or_insert(0) += 1;
    }

    let mut options = counts
        .into_iter()
        .map(|(name, count)| FilterOption {
            filter_key: name.clone(),
            label: format!("{name} ({count})"),
        })
        .collect::<Vec<_>>();
    options.sort_by(|a, b| a.filter_key.cmp(&b.filter_key));
    options
}

fn filter_display_spans(
    spans: &[DisplaySpan],
    process_filter: &str,
    function_filter: &str,
) -> Vec<DisplaySpan> {
    let mut filtered = Vec::new();
    collect_filtered_display_spans(spans, process_filter, function_filter, &mut filtered);
    if filtered.is_empty() && process_filter == "driver" && function_filter.is_empty() {
        spans.to_vec()
    } else {
        filtered
    }
}

fn collect_filtered_display_spans(
    spans: &[DisplaySpan],
    process_filter: &str,
    function_filter: &str,
    out: &mut Vec<DisplaySpan>,
) {
    for span in spans {
        let mut children = Vec::new();
        collect_filtered_display_spans(&span.children, process_filter, function_filter, &mut children);

        if process_filter_matches(span, process_filter) && function_filter_matches(span, function_filter) {
            let mut cloned = span.clone();
            cloned.children = children;
            out.push(cloned);
        } else {
            out.extend(children);
        }
    }
}

fn process_filter_matches(span: &DisplaySpan, process_filter: &str) -> bool {
    if process_filter.is_empty() || process_filter == "all" {
        return true;
    }
    if process_filter == "driver" {
        return attr_string(&span.attributes, "process_role")
            .map(|role| role.contains("driver"))
            .unwrap_or(false);
    }
    if let Some(pid_filter) = process_filter.strip_prefix("pid:") {
        return process_pid_from_attrs(&span.attributes).as_deref() == Some(pid_filter);
    }
    false
}

fn function_filter_matches(span: &DisplaySpan, function_filter: &str) -> bool {
    function_filter.is_empty() || span.name == function_filter
}

fn collect_span_process_pids(spans: &[SpanInfo]) -> HashSet<i32> {
    let mut pids = HashSet::new();
    for span in spans {
        if let Some(pid) = process_pid_from_attrs(&span.attributes)
            .and_then(|pid| pid.parse::<i32>().ok())
        {
            pids.insert(pid);
        }
        pids.extend(collect_span_process_pids(&span.children));
    }
    pids
}

fn process_pid_from_attrs(attributes: &Option<String>) -> Option<String> {
    attr_string(attributes, "pid").filter(|pid| !pid.is_empty() && pid != "0")
}

fn short_id(value: &str) -> String {
    if value.len() > 8 {
        value[..8].to_string()
    } else {
        value.to_string()
    }
}

fn process_option_rank(option: &FilterOption) -> usize {
    if option.label.contains("driver") {
        0
    } else if option.label.contains("rollout") {
        1
    } else {
        2
    }
}

fn collect_batch_options(spans: &[DisplaySpan]) -> Vec<TimelineSpan> {
    let mut rows = Vec::new();
    collect_batches(spans, &mut rows);
    rows.sort_by_key(|row| row.start);
    rows
}

fn collect_batches(spans: &[DisplaySpan], rows: &mut Vec<TimelineSpan>) {
    for span in spans {
        let is_batch = span.name == "batch"
            || is_step_parent_span(&span.name, span.kind.as_deref());
        if is_batch {
            let duration = span
                .end_timestamp
                .map(|end| (end - span.start_timestamp).max(0))
                .unwrap_or(0);
            rows.push(TimelineSpan {
                unique_key: span.unique_key.clone(),
                name: span.name.clone(),
                display_id: span.display_id.clone(),
                span_id: span.span_id,
                kind: span.kind.clone(),
                location: span.location.clone(),
                start: span.start_timestamp,
                duration,
                depth: 0,
                process_label: span.process_label.clone(),
            });
        }
        collect_batches(&span.children, rows);
    }
}

fn find_display_span<'a>(spans: &'a [DisplaySpan], unique_key: &str) -> Option<&'a DisplaySpan> {
    for span in spans {
        if span.unique_key == unique_key {
            return Some(span);
        }
        if let Some(found) = find_display_span(&span.children, unique_key) {
            return Some(found);
        }
    }
    None
}

fn cross_process_link_summary_view(spans: &[DisplaySpan]) -> CrossProcessLinkSummaryView {
    let mut summary = CrossProcessLinkSummaryView::default();
    let mut orphan_keys = HashMap::<String, usize>::new();
    collect_cross_process_link_summary(spans, false, &mut summary, &mut orphan_keys);
    let mut orphan_keys = orphan_keys.into_iter().collect::<Vec<_>>();
    orphan_keys.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    summary.orphan_keys = orphan_keys
        .into_iter()
        .take(3)
        .map(|(key, count)| format!("{key}:{count}"))
        .collect::<Vec<_>>()
        .join(", ");
    summary
}

fn collect_cross_process_link_summary(
    spans: &[DisplaySpan],
    under_logical_parent: bool,
    summary: &mut CrossProcessLinkSummaryView,
    orphan_keys: &mut HashMap<String, usize>,
) {
    for span in spans {
        let is_parent = is_display_cross_process_parent_span(span);
        let under_parent = under_logical_parent || is_parent;
        if is_parent {
            summary.parents += 1;
        }

        if is_display_cross_process_child_candidate(span) {
            if under_logical_parent {
                summary.linked += 1;
            } else {
                summary.orphan += 1;
                if let Some(key) = display_logical_step_key(span) {
                    let key = format!("{}/{}", key.rollout_id, key.step_id);
                    *orphan_keys.entry(key).or_insert(0) += 1;
                }
            }
        }

        collect_cross_process_link_summary(&span.children, under_parent, summary, orphan_keys);
    }
}

fn is_display_logical_parent_span(span: &DisplaySpan) -> bool {
    is_step_parent_span(&span.name, span.kind.as_deref())
        && !is_rollout_submit_parent_span(&span.name)
}

fn is_display_submit_parent_span(span: &DisplaySpan) -> bool {
    is_rollout_submit_parent_span(&span.name)
}

fn is_display_cross_process_parent_span(span: &DisplaySpan) -> bool {
    is_display_logical_parent_span(span) || is_display_submit_parent_span(span)
}

fn is_display_cross_process_child_candidate(span: &DisplaySpan) -> bool {
    if display_logical_step_key(span).is_none() || is_display_cross_process_parent_span(span) {
        return false;
    }

    let role = attr_string(&span.attributes, "process_role")
        .or_else(|| attr_string(&span.attributes, "actor_role"))
        .unwrap_or_default();
    is_rollout_worker_role(&role) || span.name.starts_with("custom.generate")
}

fn display_logical_step_key(span: &DisplaySpan) -> Option<LogicalStepKey> {
    let rollout_id = attr_string(&span.attributes, "rollout_id")?;
    let step_id = attr_string(&span.attributes, "step_id")?;

    if rollout_id == "-1" || step_id == "-1" {
        return None;
    }

    Some(LogicalStepKey { rollout_id, step_id })
}

fn expanded_trace_fetch_limit(limit: usize) -> usize {
    limit.saturating_mul(5).min(10_000).max(limit)
}

fn link_cross_process_spans(spans: Vec<SpanInfo>) -> Vec<SpanInfo> {
    let mut linked_roots = Vec::new();
    let mut pending_children = Vec::new();

    split_cross_process_candidates(spans, &mut linked_roots, &mut pending_children);

    pending_children.sort_by_key(|span| span.start_timestamp);
    for child in pending_children {
        if let Some(key) = logical_step_key(&child) {
            if attach_to_logical_parent(&mut linked_roots, &key, child.clone()) {
                continue;
            }
        }
        if let Some(key) = submit_link_key(&child) {
            if attach_to_submit_parent(&mut linked_roots, &key, child.clone()) {
                continue;
            }
        }
        if attach_to_time_parent(&mut linked_roots, child.clone()) {
            continue;
        }
        if let Some(rollout_id) = logical_rollout_id(&child) {
            if attach_to_single_rollout_parent(&mut linked_roots, &rollout_id, child.clone()) {
                continue;
            }
        }
        linked_roots.push(child);
    }

    sort_span_tree(&mut linked_roots);
    linked_roots
}

fn split_cross_process_candidates(
    spans: Vec<SpanInfo>,
    linked_roots: &mut Vec<SpanInfo>,
    pending_children: &mut Vec<SpanInfo>,
) {
    for mut span in spans {
        if rl_contract::is_cross_process_child_candidate(&span) {
            pending_children.push(span);
            continue;
        }

        let children = std::mem::take(&mut span.children);
        split_cross_process_candidates(children, &mut span.children, pending_children);
        linked_roots.push(span);
    }
}

fn is_logical_parent_span(span: &SpanInfo) -> bool {
    is_step_parent_span(&span.name, span.phase.as_deref())
        && !is_rollout_submit_parent_span(&span.name)
}

fn is_submit_parent_span(span: &SpanInfo) -> bool {
    is_rollout_submit_parent_span(&span.name)
}

fn submit_link_key(span: &SpanInfo) -> Option<LogicalStepKey> {
    let rollout_id = attr_string(&span.attributes, "rollout_id")?;
    let submit_step_id = attr_string(&span.attributes, "submit_step_id")?;

    if rollout_id == "-1" {
        return None;
    }

    Some(LogicalStepKey {
        rollout_id,
        step_id: submit_step_id,
    })
}

fn logical_rollout_id(span: &SpanInfo) -> Option<String> {
    let rollout_id = attr_string(&span.attributes, "rollout_id")?;
    if rollout_id == "-1" {
        None
    } else {
        Some(rollout_id)
    }
}

fn attach_to_logical_parent(spans: &mut [SpanInfo], key: &LogicalStepKey, child: SpanInfo) -> bool {
    for span in spans {
        if is_logical_parent_span(span) && logical_step_key(span).as_ref() == Some(key) {
            span.children.push(child);
            span.children.sort_by_key(|child| child.start_timestamp);
            return true;
        }

        if attach_to_logical_parent(&mut span.children, key, child.clone()) {
            return true;
        }
    }
    false
}

fn attach_to_submit_parent(spans: &mut [SpanInfo], key: &LogicalStepKey, child: SpanInfo) -> bool {
    for span in spans {
        if is_submit_parent_span(span) && submit_parent_matches(span, key) {
            span.children.push(child);
            span.children.sort_by_key(|child| child.start_timestamp);
            return true;
        }

        if attach_to_submit_parent(&mut span.children, key, child.clone()) {
            return true;
        }
    }
    false
}

fn submit_parent_matches(span: &SpanInfo, key: &LogicalStepKey) -> bool {
    let rollout_id = attr_string(&span.attributes, "rollout_id");
    let submit_step_id = attr_string(&span.attributes, "submit_step_id")
        .or_else(|| attr_string(&span.attributes, "step_id"));
    rollout_id.as_deref() == Some(key.rollout_id.as_str())
        && submit_step_id.as_deref() == Some(key.step_id.as_str())
}

fn attach_to_single_rollout_parent(spans: &mut [SpanInfo], rollout_id: &str, child: SpanInfo) -> bool {
    let mut matches = Vec::<Vec<usize>>::new();
    let mut current_path = Vec::<usize>::new();
    find_rollout_parent_paths(spans, rollout_id, &mut current_path, &mut matches);

    if matches.len() != 1 {
        return false;
    }

    if let Some(parent) = get_span_mut_by_path(spans, &matches[0]) {
        parent.children.push(child);
        parent.children.sort_by_key(|child| child.start_timestamp);
        return true;
    }

    false
}

fn find_rollout_parent_paths(
    spans: &[SpanInfo],
    rollout_id: &str,
    current_path: &mut Vec<usize>,
    matches: &mut Vec<Vec<usize>>,
) {
    for (index, span) in spans.iter().enumerate() {
        current_path.push(index);

        if is_logical_parent_span(span) && logical_rollout_id(span).as_deref() == Some(rollout_id) {
            matches.push(current_path.clone());
        }

        find_rollout_parent_paths(&span.children, rollout_id, current_path, matches);
        current_path.pop();
    }
}

fn attach_to_time_parent(spans: &mut [SpanInfo], child: SpanInfo) -> bool {
    let mut best_path = Vec::<usize>::new();
    let mut current_path = Vec::<usize>::new();
    let mut best_duration: Option<i64> = None;
    find_time_parent_path(spans, &child, &mut current_path, &mut best_path, &mut best_duration);

    if best_path.is_empty() {
        return false;
    }

    if let Some(parent) = get_span_mut_by_path(spans, &best_path) {
        parent.children.push(child);
        parent.children.sort_by_key(|child| child.start_timestamp);
        return true;
    }

    false
}

fn find_time_parent_path(
    spans: &[SpanInfo],
    child: &SpanInfo,
    current_path: &mut Vec<usize>,
    best_path: &mut Vec<usize>,
    best_duration: &mut Option<i64>,
) {
    for (index, span) in spans.iter().enumerate() {
        current_path.push(index);

        if is_logical_parent_span(span) && span_contains_child(span, child) {
            let duration = span
                .end_timestamp
                .map(|end| end - span.start_timestamp)
                .unwrap_or(i64::MAX);
            if best_duration.map(|best| duration < best).unwrap_or(true) {
                *best_duration = Some(duration);
                *best_path = current_path.clone();
            }
        }

        find_time_parent_path(&span.children, child, current_path, best_path, best_duration);
        current_path.pop();
    }
}

fn span_contains_child(parent: &SpanInfo, child: &SpanInfo) -> bool {
    let child_end = child.end_timestamp.unwrap_or(child.start_timestamp);
    let parent_end = parent.end_timestamp.unwrap_or(i64::MAX);
    child.start_timestamp >= parent.start_timestamp && child_end <= parent_end
}

fn get_span_mut_by_path<'a>(spans: &'a mut [SpanInfo], path: &[usize]) -> Option<&'a mut SpanInfo> {
    let (first, rest) = path.split_first()?;
    let span = spans.get_mut(*first)?;
    if rest.is_empty() {
        Some(span)
    } else {
        get_span_mut_by_path(&mut span.children, rest)
    }
}

fn sort_span_tree(spans: &mut [SpanInfo]) {
    spans.sort_by_key(|span| span.start_timestamp);
    for span in spans {
        sort_span_tree(&mut span.children);
    }
}

fn process_label(attributes: &Option<String>, fallback_trace_id: i64) -> String {
    let role = attr_string(attributes, "process_role").unwrap_or_else(|| "process".to_string());
    let pid = attr_string(attributes, "pid");
    match pid {
        Some(pid) if !pid.is_empty() => format!("{role}:{pid}"),
        _ => format!("{role}:{fallback_trace_id}"),
    }
}

fn attr_string(attributes: &Option<String>, key: &str) -> Option<String> {
    let attrs = attributes.as_ref()?;
    let value = serde_json::from_str::<serde_json::Value>(attrs).ok()?;
    let raw = value.get(key)?;
    match raw {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn format_duration_ns(duration_ns: i64) -> String {
    let duration_ns = duration_ns.max(0) as f64;
    if duration_ns >= 1_000_000_000.0 {
        format!("{:.3}s", duration_ns / 1_000_000_000.0)
    } else if duration_ns >= 1_000_000.0 {
        format!("{:.3}ms", duration_ns / 1_000_000.0)
    } else if duration_ns >= 1_000.0 {
        format!("{:.3}us", duration_ns / 1_000.0)
    } else {
        format!("{:.0}ns", duration_ns)
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
