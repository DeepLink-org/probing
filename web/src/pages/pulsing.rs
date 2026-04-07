use std::collections::{HashMap, HashSet};

use dioxus::prelude::*;
use probing_proto::prelude::{DataFrame, Ele};

use crate::api::ApiClient;
use crate::components::card::Card;
use crate::components::colors::colors;
use crate::components::common::{EmptyState, ErrorState, LoadingState};
use crate::components::dataframe_view::DataFrameView;
use crate::components::page::{PageContainer, PageTitle};
use crate::hooks::use_api;

#[component]
pub fn Pulsing() -> Element {
    let actors = use_api(|| {
        let c = ApiClient::new();
        async move { c.fetch_pulsing_actors().await }
    });
    let span_count = use_api(|| {
        let c = ApiClient::new();
        async move { c.fetch_pulsing_span_count().await }
    });
    let spans = use_api(|| {
        let c = ApiClient::new();
        async move { c.fetch_pulsing_spans().await }
    });
    let metrics = use_api(|| {
        let c = ApiClient::new();
        async move { c.fetch_pulsing_metrics().await }
    });
    let members = use_api(|| {
        let c = ApiClient::new();
        async move { c.fetch_pulsing_members().await }
    });

    let expanded_traces = use_signal(|| -> HashSet<usize> {
        (0..3).collect()
    });

    rsx! {
        PageContainer {
            PageTitle {
                title: "Pulsing Actors".to_string(),
                subtitle: Some("Actor system overview, spans, metrics & membership".to_string()),
                icon: Some(&icondata::AiDeploymentUnitOutlined),
            }
            {summary_row(&actors, &span_count, &members)}

            Card {
                title: "Actors",
                content_class: Some(""),
                {query_result(&actors, "No actors registered")}
            }

            Card {
                title: "Trace Timeline",
                content_class: Some(""),
                {trace_timeline(&spans, expanded_traces)}
            }

            Card {
                title: "Metrics (latest 100)",
                content_class: Some(""),
                {query_result(&metrics, "No metrics recorded yet")}
            }

            Card {
                title: "Cluster Members",
                content_class: Some(""),
                {query_result(&members, "No cluster members (standalone mode)")}
            }
        }
    }
}

// ── helpers ──────────────────────────────────────────────────────────

fn ele_str(e: &Ele) -> String {
    match e {
        Ele::Text(s) => s.to_string(),
        Ele::I64(v) => v.to_string(),
        Ele::I32(v) => v.to_string(),
        Ele::F64(v) => v.to_string(),
        _ => String::new(),
    }
}

fn ele_i64(e: &Ele) -> i64 {
    match e {
        Ele::I64(v) => *v,
        Ele::I32(v) => *v as i64,
        _ => 0,
    }
}

fn format_duration_us(us: f64) -> String {
    if us < 1_000.0 {
        format!("{:.0}µs", us)
    } else if us < 1_000_000.0 {
        format!("{:.1}ms", us / 1_000.0)
    } else {
        format!("{:.2}s", us / 1_000_000.0)
    }
}

/// Format a microsecond-epoch timestamp as `HH:MM:SS.mmm` (UTC).
fn format_timestamp_us(us: i64) -> String {
    let total_sec = us / 1_000_000;
    let millis = (us % 1_000_000) / 1_000;
    let h = (total_sec / 3600) % 24;
    let m = (total_sec / 60) % 60;
    let s = total_sec % 60;
    format!("{:02}:{:02}:{:02}.{:03}", h, m, s, millis)
}

fn count_rows(state: &crate::hooks::ApiState<DataFrame>) -> String {
    match state.data.read().as_ref() {
        Some(Ok(df)) => df.cols.first().map(|c| c.len()).unwrap_or(0).to_string(),
        _ if state.is_loading() => "…".into(),
        _ => "-".into(),
    }
}

fn scalar_count(state: &crate::hooks::ApiState<DataFrame>) -> String {
    match state.data.read().as_ref() {
        Some(Ok(df)) => {
            if let Some(col) = df.cols.first() {
                match col.get(0) {
                    Ele::I64(v) => v.to_string(),
                    Ele::I32(v) => v.to_string(),
                    _ => "0".into(),
                }
            } else {
                "0".into()
            }
        }
        _ if state.is_loading() => "…".into(),
        _ => "-".into(),
    }
}

// ── summary ──────────────────────────────────────────────────────────

fn summary_row(
    actors: &crate::hooks::ApiState<DataFrame>,
    span_count: &crate::hooks::ApiState<DataFrame>,
    members: &crate::hooks::ApiState<DataFrame>,
) -> Element {
    rsx! {
        div { class: "grid grid-cols-1 sm:grid-cols-3 gap-4",
            StatCard { label: "Live Actors", value: count_rows(actors) }
            StatCard { label: "Spans Captured", value: scalar_count(span_count) }
            StatCard { label: "Cluster Members", value: count_rows(members) }
        }
    }
}

#[component]
fn StatCard(label: &'static str, value: String) -> Element {
    rsx! {
        div {
            class: "bg-white border border-gray-200 rounded-lg px-5 py-4 shadow-sm",
            p { class: "text-xs font-medium text-gray-500 uppercase tracking-wide", "{label}" }
            p { class: format!("text-2xl font-bold text-{} mt-1", colors::PRIMARY), "{value}" }
        }
    }
}

fn query_result(state: &crate::hooks::ApiState<DataFrame>, empty_msg: &str) -> Element {
    if state.is_loading() {
        return rsx! { LoadingState { message: Some("Loading…".to_string()) } };
    }
    match state.data.read().as_ref() {
        Some(Ok(df)) if df.cols.first().map(|c| c.len()).unwrap_or(0) > 0 => {
            rsx! { DataFrameView { df: df.clone(), on_row_click: None } }
        }
        Some(Err(e)) => rsx! { ErrorState { error: e.display_message(), title: None } },
        _ => rsx! { EmptyState { message: empty_msg.to_string() } },
    }
}

// ── Span data model ──────────────────────────────────────────────────

#[derive(Clone)]
struct SpanRow {
    trace_id: String,
    span_id: String,
    parent_span_id: String,
    name: String,
    kind: String,
    start_us: i64,
    end_us: i64,
    duration_us: i64,
    status: String,
    actor: String,
}

fn parse_spans(df: &DataFrame) -> Vec<SpanRow> {
    let col = |name: &str| df.names.iter().position(|n| n == name);
    let ci_tid = col("trace_id");
    let ci_sid = col("span_id");
    let ci_pid = col("parent_span_id");
    let ci_name = col("name");
    let ci_kind = col("kind");
    let ci_start = col("start_us");
    let ci_end = col("end_us");
    let ci_dur = col("duration_us");
    let ci_status = col("status_code");
    let ci_actor = col("attr_actor_name");
    let _ci_op = col("attr_pulsing_op");

    let nrows = df.cols.first().map(|c| c.len()).unwrap_or(0);
    (0..nrows)
        .map(|i| SpanRow {
            trace_id: ci_tid.map(|c| ele_str(&df.cols[c].get(i))).unwrap_or_default(),
            span_id: ci_sid.map(|c| ele_str(&df.cols[c].get(i))).unwrap_or_default(),
            parent_span_id: ci_pid.map(|c| ele_str(&df.cols[c].get(i))).unwrap_or_default(),
            name: ci_name.map(|c| ele_str(&df.cols[c].get(i))).unwrap_or_default(),
            kind: ci_kind.map(|c| ele_str(&df.cols[c].get(i))).unwrap_or_default(),
            start_us: ci_start.map(|c| ele_i64(&df.cols[c].get(i))).unwrap_or(0),
            end_us: ci_end.map(|c| ele_i64(&df.cols[c].get(i))).unwrap_or(0),
            duration_us: ci_dur.map(|c| ele_i64(&df.cols[c].get(i))).unwrap_or(0),
            status: ci_status.map(|c| ele_str(&df.cols[c].get(i))).unwrap_or_default(),
            actor: ci_actor.map(|c| ele_str(&df.cols[c].get(i))).unwrap_or_default(),
        })
        .collect()
}

struct FlatSpan {
    span: SpanRow,
    depth: usize,
    /// Actor name of the parent span (the caller), empty for root spans.
    caller: String,
    /// Self time = wall time minus direct children's duration (clamped ≥ 0).
    self_time_us: i64,
}

/// Build tree-ordered flat list: for each trace, DFS from root spans.
fn flatten_traces(spans: &[SpanRow]) -> Vec<(String, Vec<FlatSpan>)> {
    let mut by_trace: HashMap<&str, Vec<&SpanRow>> = HashMap::new();
    for s in spans {
        by_trace.entry(&s.trace_id).or_default().push(s);
    }

    let mut traces: Vec<(String, Vec<FlatSpan>)> = Vec::new();

    for (tid, group) in &by_trace {
        let children_of: HashMap<&str, Vec<&SpanRow>> = {
            let mut m: HashMap<&str, Vec<&SpanRow>> = HashMap::new();
            for s in group {
                m.entry(s.parent_span_id.as_str()).or_default().push(s);
            }
            m
        };
        let span_ids: std::collections::HashSet<&str> =
            group.iter().map(|s| s.span_id.as_str()).collect();

        let mut roots: Vec<&&SpanRow> = group
            .iter()
            .filter(|s| {
                s.parent_span_id.is_empty()
                    || !span_ids.contains(s.parent_span_id.as_str())
            })
            .collect();
        roots.sort_by_key(|s| s.start_us);

        let mut flat = Vec::new();
        // Stack: (span, depth, parent_actor_name)
        let mut stack: Vec<(&SpanRow, usize, String)> = roots
            .into_iter()
            .rev()
            .map(|s| (*s, 0, String::new()))
            .collect();

        while let Some((span, depth, caller)) = stack.pop() {
            let this_actor = span.actor.clone();
            let kids = children_of.get(span.span_id.as_str());
            let children_dur: i64 = kids
                .map(|k| k.iter().map(|c| c.duration_us).sum())
                .unwrap_or(0);
            let self_time_us = (span.duration_us - children_dur).max(0);

            flat.push(FlatSpan {
                span: span.clone(),
                depth,
                caller,
                self_time_us,
            });
            if let Some(kids) = kids {
                let mut sorted: Vec<&&SpanRow> = kids.iter().collect();
                sorted.sort_by_key(|s| std::cmp::Reverse(s.start_us));
                for kid in sorted {
                    stack.push((kid, depth + 1, this_actor.clone()));
                }
            }
        }
        if !flat.is_empty() {
            traces.push((tid.to_string(), flat));
        }
    }

    traces.sort_by(|a, b| {
        let a_start = a.1.first().map(|f| f.span.start_us).unwrap_or(0);
        let b_start = b.1.first().map(|f| f.span.start_us).unwrap_or(0);
        b_start.cmp(&a_start)
    });
    traces
}

// ── Trace Timeline rendering ─────────────────────────────────────────

fn trace_timeline(
    state: &crate::hooks::ApiState<DataFrame>,
    expanded: Signal<HashSet<usize>>,
) -> Element {
    if state.is_loading() {
        return rsx! { LoadingState { message: Some("Loading spans…".to_string()) } };
    }
    let data_ref = state.data.read();
    let df = match data_ref.as_ref() {
        Some(Ok(df)) => df,
        Some(Err(e)) => return rsx! { ErrorState { error: e.display_message(), title: None } },
        None => return rsx! { EmptyState { message: "No spans".to_string() } },
    };

    let spans = parse_spans(df);
    if spans.is_empty() {
        return rsx! { EmptyState { message: "No spans recorded yet".to_string() } };
    }

    let traces = flatten_traces(&spans);
    let max_traces = 30;

    rsx! {
        div { class: "space-y-3",
            // Legend
            div { class: "flex items-center gap-5 px-1 py-1.5 text-xs text-gray-500",
                div { class: "flex items-center gap-1.5",
                    div { class: "w-5 h-3 rounded bg-emerald-500" }
                    span { "Self time — span 自身执行" }
                }
                div { class: "flex items-center gap-1.5",
                    div { class: "w-5 h-3 rounded bg-emerald-200" }
                    span { "Wall time — 含子调用的总耗时" }
                }
                div { class: "flex items-center gap-1.5",
                    div { class: "w-5 h-3 rounded bg-red-500" }
                    span { "Error" }
                }
            }
            for (idx, (trace_id, flat_spans)) in traces.iter().enumerate().take(max_traces) {
                {trace_block(trace_id, flat_spans, idx, expanded)}
            }
            if traces.len() > max_traces {
                p { class: "text-sm text-gray-400 text-center py-2",
                    "Showing {max_traces} of {traces.len()} traces"
                }
            }
        }
    }
}

fn trace_block(
    trace_id: &str,
    flat_spans: &[FlatSpan],
    idx: usize,
    expanded: Signal<HashSet<usize>>,
) -> Element {
    if flat_spans.is_empty() {
        return rsx! {};
    }

    let is_open = expanded.read().contains(&idx);
    let trace_start = flat_spans.iter().map(|f| f.span.start_us).min().unwrap_or(0);
    let trace_end = flat_spans.iter().map(|f| f.span.end_us).max().unwrap_or(1);
    let trace_range = (trace_end - trace_start).max(1) as f64;
    let trace_dur_label = format_duration_us(trace_range);
    let root_name = root_label(&flat_spans[0]);
    let short_tid = if trace_id.len() > 8 {
        &trace_id[..8]
    } else {
        trace_id
    };

    let max_depth = flat_spans.iter().map(|f| f.depth).max().unwrap_or(0);
    let depth_info = if max_depth > 0 {
        format!("depth {max_depth}")
    } else {
        String::new()
    };

    let chevron_class = if is_open {
        "rotate-90"
    } else {
        "rotate-0"
    };

    let mut sig = expanded;

    rsx! {
        div { class: "border border-gray-200 rounded-lg overflow-hidden",
            // Clickable trace header
            div {
                class: "bg-gray-50 px-4 py-2.5 border-b border-gray-100 flex items-center justify-between cursor-pointer hover:bg-gray-100/80 transition-colors select-none",
                onclick: move |_| {
                    let mut set = sig.write();
                    if set.contains(&idx) {
                        set.remove(&idx);
                    } else {
                        set.insert(idx);
                    }
                },
                div { class: "flex items-center gap-2 min-w-0",
                    // Chevron
                    svg {
                        class: "w-3.5 h-3.5 text-gray-400 flex-shrink-0 transition-transform duration-150 {chevron_class}",
                        fill: "none",
                        stroke: "currentColor",
                        view_box: "0 0 24 24",
                        path {
                            stroke_linecap: "round",
                            stroke_linejoin: "round",
                            stroke_width: "2",
                            d: "M9 5l7 7-7 7",
                        }
                    }
                    span { class: "text-sm font-semibold text-gray-800 truncate", "{root_name}" }
                    span { class: "text-xs text-gray-400 font-mono flex-shrink-0", "{short_tid}…" }
                }
                div { class: "flex items-center gap-3 flex-shrink-0 ml-3",
                    if !depth_info.is_empty() {
                        span { class: "text-xs text-gray-400", "{depth_info}" }
                    }
                    span { class: "text-xs text-gray-500", "{flat_spans.len()} spans" }
                    span { class: "text-xs font-medium text-gray-600", "{trace_dur_label}" }
                }
            }
            // Span rows (only if expanded)
            if is_open {
                div { class: "divide-y divide-gray-100",
                    for (i, fs) in flat_spans.iter().enumerate() {
                        {span_row(fs, trace_start, trace_range, i)}
                    }
                }
            }
        }
    }
}

/// Root label for the trace header.
fn root_label(fs: &FlatSpan) -> String {
    if fs.span.actor.is_empty() {
        fs.span.name.clone()
    } else {
        fs.span.actor.clone()
    }
}

/// Row label: "caller → callee" for child spans, just actor for root.
fn row_label(fs: &FlatSpan) -> String {
    let callee = if fs.span.actor.is_empty() {
        &fs.span.name
    } else {
        &fs.span.actor
    };
    if fs.caller.is_empty() || fs.caller == *callee {
        callee.to_string()
    } else {
        format!("{} → {}", fs.caller, callee)
    }
}

fn span_tooltip(s: &SpanRow, caller: &str, self_time_us: i64) -> String {
    let wall = format_duration_us(s.duration_us as f64);
    let self_t = format_duration_us(self_time_us as f64);
    let start = format_timestamp_us(s.start_us);
    let end = format_timestamp_us(s.end_us);
    let caller_part = if caller.is_empty() {
        "root".to_string()
    } else {
        format!("caller: {caller}")
    };
    format!(
        "{}\nspan: {}\n{}\nstart: {} → end: {}\nwall: {} | self: {}\nstatus: {} | kind: {}\nspan_id: {}",
        s.actor, s.name, caller_part, start, end, wall, self_t, s.status, s.kind, s.span_id,
    )
}

fn span_row(fs: &FlatSpan, trace_start: i64, trace_range: f64, idx: usize) -> Element {
    let s = &fs.span;
    let indent_px = fs.depth * 24;
    let left_pct = ((s.start_us - trace_start) as f64 / trace_range * 100.0).max(0.0);
    let wall_width_pct = (s.duration_us as f64 / trace_range * 100.0).max(0.5);
    let self_ratio = if s.duration_us > 0 {
        fs.self_time_us as f64 / s.duration_us as f64
    } else {
        1.0
    };
    let wall_label = format_duration_us(s.duration_us as f64);
    let self_label = format_duration_us(fs.self_time_us as f64);
    let start_label = format_timestamp_us(s.start_us);
    let end_label = format_timestamp_us(s.end_us);

    let (wall_color, self_color) = match s.status.as_str() {
        "ok" => ("bg-emerald-200", "bg-emerald-500"),
        "error" => ("bg-red-200", "bg-red-500"),
        _ => ("bg-blue-200", "bg-blue-500"),
    };
    let row_bg = if idx % 2 == 0 { "bg-white" } else { "bg-gray-50/30" };

    let label = row_label(fs);
    let tooltip = span_tooltip(s, &fs.caller, fs.self_time_us);

    let time_label = if fs.self_time_us < s.duration_us {
        format!("{wall_label} (self {self_label})")
    } else {
        wall_label.clone()
    };

    rsx! {
        div {
            class: "flex items-center {row_bg} hover:bg-blue-50/40 transition-colors min-h-[34px]",
            title: "{tooltip}",
            div {
                class: "flex-shrink-0 w-[360px] px-3 py-1 text-sm border-r border-gray-100 flex items-center min-w-0",
                style: "padding-left: {indent_px + 12}px",
                if fs.depth > 0 {
                    span { class: "text-gray-300 mr-1.5 flex-shrink-0", "└" }
                }
                span {
                    class: "font-medium text-gray-700 truncate",
                    title: "{label}",
                    "{label}"
                }
            }
            div { class: "flex-1 px-2 py-1 relative h-[28px]",
                // Wall time bar (lighter)
                div {
                    class: "absolute top-1 h-[20px] rounded {wall_color} cursor-default",
                    style: "left: {left_pct:.2}%; width: {wall_width_pct:.2}%;",
                    title: "{s.name}\n{start_label} → {end_label}\nwall: {wall_label} | self: {self_label}\n[{s.status}]",
                }
                // Self time bar (solid, overlaid from left edge)
                div {
                    class: "absolute top-1 h-[20px] rounded-l {self_color} pointer-events-none",
                    style: "left: {left_pct:.2}%; width: calc({wall_width_pct:.2}% * {self_ratio:.4});",
                }
                // Duration label
                div {
                    class: "absolute top-1 h-[20px] flex items-center text-[10px] text-gray-500 pointer-events-none whitespace-nowrap",
                    style: "left: calc({left_pct:.2}% + {wall_width_pct:.2}% + 4px);",
                    "{time_label}"
                }
            }
        }
    }
}
