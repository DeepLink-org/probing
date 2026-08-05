use std::collections::{BTreeMap, HashSet};

use dioxus::prelude::*;

use crate::api::{ApiClient, SpanInfo};
use crate::components::span_timeline::{
    span_self_ns, span_total_ns, SpanSummaryBar, SpanSummaryHeader, TraceTimeWindow,
};
use crate::state::investigation::{set_trace_context, InvestigationContext, INVESTIGATION_CONTEXT};
use crate::state::profiling::{SPANS_TREE_LIMIT, SPANS_TREE_RELOAD};

use super::super::components::{
    ActionButton, EvidenceSection, EvidenceSurface, FilterInput, InlineNotice, LoadingPanel,
    NoticeTone, UnavailablePanel, WorkspacePage,
};

#[component]
pub fn SpansPage() -> Element {
    let limit = *SPANS_TREE_LIMIT.read();
    let spans = use_resource(move || {
        let reload = *SPANS_TREE_RELOAD.read();
        let request_limit = *SPANS_TREE_LIMIT.read();
        async move {
            let _ = reload;
            ApiClient::new().get_span_tree(Some(request_limit)).await
        }
    });
    let mut query = use_signal(|| {
        INVESTIGATION_CONTEXT
            .read()
            .span_name
            .clone()
            .unwrap_or_default()
    });
    let mut expansion = use_signal(|| Expansion::Roots);
    let mut tree_version = use_signal(|| 0_u32);
    let context = INVESTIGATION_CONTEXT.read().clone();

    rsx! {
        WorkspacePage {
            title: "Tracing".to_string(),
            subtitle: "Hierarchical spans from trace_event; row summaries remain visible while branches are collapsed.".to_string(),
            actions: Some(rsx! {
                    FilterInput {
                        value: query(),
                        placeholder: "Find span, phase, or location".to_string(),
                        oninput: move |value| query.set(value),
                    }
                    ActionButton {
                        label: "Expand all".to_string(),
                        onclick: move |_| { expansion.set(Expansion::All); tree_version += 1; },
                    }
                    ActionButton {
                        label: "Collapse all".to_string(),
                        onclick: move |_| { expansion.set(Expansion::None); tree_version += 1; },
                    }
                }),
            match spans.read().clone() {
                None => rsx! { LoadingPanel { label: "Loading span hierarchy".to_string() } },
                Some(Err(error)) => rsx! { UnavailablePanel {
                    label: "Span evidence unavailable".to_string(),
                    detail: error.display_message(),
                }},
                Some(Ok(spans)) => rsx! { TraceEvidence {
                    key: "{tree_version}",
                    spans,
                    context,
                    query: query(),
                    expansion: expansion(),
                    limit,
                }},
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Expansion {
    None,
    Roots,
    All,
}

#[component]
fn TraceEvidence(
    spans: Vec<SpanInfo>,
    context: InvestigationContext,
    query: String,
    expansion: Expansion,
    limit: usize,
) -> Element {
    let context_filter_active = trace_context_active(&context);
    let contextual = filter_span_context_tree(&spans, &context);
    let context_match_empty = contextual.is_empty();
    let filtered = filter_span_tree(&contextual, &query);
    let total = count_spans(&filtered);
    let events = count_events(&filtered);
    let active = count_active(&filtered);
    let completed = total.saturating_sub(active);
    let threads = thread_count(&filtered);
    let window = TraceTimeWindow::from_spans(&filtered);
    let duration = format_ns(window.range_ns());
    let groups = group_traces(filtered.clone());

    rsx! {
        EvidenceSurface {
            EvidenceSection {
                title: "Trace hierarchy".to_string(),
                subtitle: Some("Select a trace or span to pin it; timing and structure remain visible while branches are collapsed.".to_string()),
                body_class: "p-0".to_string(),
                if filtered.is_empty() {
                    div { class: "p-4",
                        UnavailablePanel {
                            label: "No matching spans".to_string(),
                            detail: if spans.is_empty() {
                                "No trace_event span samples were returned.".to_string()
                            } else if context_filter_active && context_match_empty {
                                "The current process trace buffer has no span matching the pinned rank, step, trace, or thread coordinates.".to_string()
                            } else {
                                "Change the search text or reload a larger window.".to_string()
                            },
                        }
                    }
                } else {
                    if active > 0 {
                        div { class: "p-3 pb-0",
                            InlineNotice {
                                title: format!("{active} span(s) still in progress"),
                                detail: "Active spans remain visible for structure and position, but their elapsed wall time is not reported as a completed duration.".to_string(),
                                tone: NoticeTone::Warning,
                            }
                        }
                    }
                    div { class: "border-b border-gray-200 bg-gray-50/70 px-4 py-2 text-xs text-gray-600",
                        div { class: "flex flex-wrap items-center gap-x-3 gap-y-1",
                            span { class: "font-medium text-gray-900", "{groups.len()} traces" }
                            span { "{filtered.len()} roots · {total} spans" }
                            span { "{threads} threads · {events} events" }
                            span { "{completed} completed · {active} active" }
                            span { class: "font-mono tabular-nums", "{duration} window" }
                            span { class: "text-gray-500", "request limit {limit}" }
                        }
                    }
                    div { class: "overflow-x-auto pb-2",
                        SpanSummaryHeader { window }
                        for group in groups {
                            TraceGroupView { group, window, expansion }
                        }
                    }
                }
            }
        }
    }
}

fn filter_span_context_tree(spans: &[SpanInfo], context: &InvestigationContext) -> Vec<SpanInfo> {
    if !trace_context_active(context) {
        return spans.to_vec();
    }

    spans
        .iter()
        .filter_map(|span| {
            if span_matches_context(span, context) {
                return Some(span.clone());
            }
            let mut kept = span.clone();
            kept.children = filter_span_context_tree(&span.children, context);
            (!kept.children.is_empty()).then_some(kept)
        })
        .collect()
}

fn trace_context_active(context: &InvestigationContext) -> bool {
    context.tid.is_some()
        || context.rank.is_some()
        || context.trace_id.is_some()
        || context.span_name.is_some()
        || context.local_step.is_some()
}

fn span_matches_context(span: &SpanInfo, context: &InvestigationContext) -> bool {
    if context
        .trace_id
        .is_some_and(|trace_id| span.trace_id != trace_id)
    {
        return false;
    }
    if context
        .tid
        .is_some_and(|tid| i64::from(tid) != span.thread_id)
    {
        return false;
    }
    if context
        .span_name
        .as_deref()
        .is_some_and(|name| name != span.name)
    {
        return false;
    }
    if context
        .rank
        .is_some_and(|rank| span_attribute_i64(span, "rank") != Some(i64::from(rank)))
    {
        return false;
    }
    if context
        .local_step
        .is_some_and(|step| span_attribute_i64(span, "local_step") != Some(step))
    {
        return false;
    }
    true
}

fn span_attribute_i64(span: &SpanInfo, key: &str) -> Option<i64> {
    let value: serde_json::Value = serde_json::from_str(span.attributes.as_deref()?).ok()?;
    value.get(key).and_then(|value| {
        value
            .as_i64()
            .or_else(|| value.as_u64().and_then(|value| i64::try_from(value).ok()))
            .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
    })
}

#[derive(Clone, Debug, PartialEq)]
struct TraceGroup {
    trace_id: i64,
    roots: Vec<SpanInfo>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct TraceGroupSummary {
    roots: usize,
    spans: usize,
    threads: usize,
    events: usize,
    active: usize,
    start_ns: i64,
    end_ns: i64,
    self_ns: i64,
    covered_ns: i64,
}

impl TraceGroupSummary {
    fn from_roots(roots: &[SpanInfo], window: TraceTimeWindow) -> Self {
        let mut thread_ids = HashSet::new();
        collect_threads(roots, &mut thread_ids);
        let start_ns = roots
            .iter()
            .map(|span| span.start_timestamp)
            .min()
            .unwrap_or(window.start_ns);
        let end_ns = roots
            .iter()
            .map(|span| span.end_timestamp.unwrap_or(window.end_ns))
            .max()
            .unwrap_or(window.end_ns);
        Self {
            roots: roots.len(),
            spans: count_spans(roots),
            threads: thread_ids.len(),
            events: count_events(roots),
            active: count_active(roots),
            start_ns,
            end_ns,
            self_ns: sum_self_ns(roots),
            covered_ns: covered_root_ns(roots, window),
        }
    }

    fn window_ns(self) -> i64 {
        (self.end_ns - self.start_ns).max(0)
    }

    fn cover_pct(self, window: TraceTimeWindow) -> f64 {
        self.covered_ns as f64 / window.range_ns() as f64 * 100.0
    }
}

#[component]
fn TraceGroupView(group: TraceGroup, window: TraceTimeWindow, expansion: Expansion) -> Element {
    let summary = TraceGroupSummary::from_roots(&group.roots, window);
    let open = expansion != Expansion::None;
    let group_window = if summary.active > 0 {
        "In progress".to_string()
    } else {
        format_ns(summary.window_ns())
    };
    let self_time = if summary.active > 0 {
        "—".to_string()
    } else {
        format_ns(summary.self_ns)
    };
    let cover = (summary.active == 0).then(|| summary.cover_pct(window));
    let cover_label = cover
        .map(|value| format!("{value:.1}%"))
        .unwrap_or_else(|| "—".to_string());
    let selected = INVESTIGATION_CONTEXT.read().trace_id == Some(group.trace_id);
    let trace_class = if selected {
        "group mt-3 min-w-[960px] overflow-hidden border-y border-blue-300 bg-white ring-1 ring-inset ring-blue-200"
    } else {
        "group mt-3 min-w-[960px] overflow-hidden border-y border-gray-200 bg-white"
    };
    let trace_id = group.trace_id;

    rsx! {
        details { class: "{trace_class}", open,
            summary {
                class: "flex cursor-pointer list-none items-center bg-gray-50/70 hover:bg-gray-100/80 focus:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-blue-600",
                aria_label: "Trace {trace_id}, {summary.spans} spans, {summary.threads} threads, total {group_window}, self {self_time}, cover {cover_label}",
                onclick: move |_| set_trace_context(trace_id, None, None),
                div { class: "w-[38%] min-w-[320px] shrink-0 px-4 py-2.5",
                    div { class: "flex min-w-0 items-center gap-2",
                        span { class: "text-xs text-gray-500 transition-transform group-open:rotate-90", aria_hidden: "true", "▶" }
                        span { class: "font-mono text-xs font-semibold text-gray-900", "trace {group.trace_id}" }
                        span { class: "truncate text-xs text-gray-500",
                            "{summary.roots} roots · {summary.spans} spans · {summary.threads} threads"
                        }
                        if summary.active > 0 {
                            span { class: "shrink-0 rounded bg-amber-50 px-1.5 py-0.5 text-xs font-medium text-amber-700", "{summary.active} active" }
                        }
                        if selected {
                            span { class: "shrink-0 rounded bg-blue-100 px-1.5 py-0.5 text-xs font-semibold text-blue-800", "Selected" }
                        }
                    }
                    div { class: "ml-5 mt-0.5 text-xs text-gray-500", "{summary.events} events" }
                }
                TraceGroupBar { roots: group.roots.clone(), window }
                div { class: "w-24 shrink-0 px-3 py-2 text-right font-mono text-xs text-gray-700", "{group_window}" }
                div {
                    class: "w-24 shrink-0 px-3 py-2 text-right font-mono text-xs text-blue-700",
                    title: "Sum of span self time; concurrent threads may overlap",
                    "{self_time}"
                }
                div { class: "w-16 shrink-0 px-3 py-2 text-right font-mono text-xs text-gray-500", "{cover_label}" }
            }
            div { class: "border-t border-gray-200",
                for span in group.roots {
                    TraceSpanRow { span, window, depth: 0, expansion }
                }
            }
        }
    }
}

#[component]
fn TraceGroupBar(roots: Vec<SpanInfo>, window: TraceTimeWindow) -> Element {
    rsx! {
        div { class: "min-w-[260px] flex-1 px-3 py-2", title: "Root span occupancy in the shared trace window",
            div { class: "relative h-4 overflow-hidden rounded-sm border border-gray-200 bg-gray-50",
                for root in roots {
                    {
                        let left = (root.start_timestamp - window.start_ns) as f64
                            / window.range_ns() as f64
                            * 100.0;
                        let width = span_total_ns(&root, window) as f64
                            / window.range_ns() as f64
                            * 100.0;
                        rsx! {
                            div {
                                class: if root.end_timestamp.is_none() { "absolute inset-y-[3px] min-w-[3px] rounded-sm bg-amber-300/80 ring-1 ring-white/80" } else { "absolute inset-y-[3px] min-w-[3px] rounded-sm bg-blue-400/70 ring-1 ring-white/80" },
                                style: "left: {left:.4}%; width: max({width:.4}%, 3px);",
                                title: if root.end_timestamp.is_none() { format!("{} · in progress", root.name) } else { root.name.clone() },
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn TraceSpanRow(
    span: SpanInfo,
    window: TraceTimeWindow,
    depth: usize,
    expansion: Expansion,
) -> Element {
    let children = span.children.clone();
    let events = span.events.clone();
    let child_count = count_spans(&children);
    let event_count = events.len() + count_events(&children);
    let active = span.end_timestamp.is_none();
    let total = span_total_label(&span, window);
    let self_time = span_self_ns(&span)
        .map(format_ns)
        .unwrap_or_else(|| "—".to_string());
    let cover = span_cover_label(&span, window);
    let indent_style = format!("padding-left: {}px", 16 + depth * 14);
    let open = expansion == Expansion::All;
    let phase = span.phase.clone().unwrap_or_else(|| "span".to_string());
    let location = span.location.clone();
    let selected_context = INVESTIGATION_CONTEXT.read().clone();
    let selected = selected_context.trace_id == Some(span.trace_id)
        && selected_context.span_name.as_deref() == Some(span.name.as_str())
        && selected_context.tid == i32::try_from(span.thread_id).ok();
    let row_class = if selected {
        "group min-w-[960px] border-b border-blue-100 bg-blue-50/50"
    } else {
        "group min-w-[960px] border-b border-gray-100"
    };
    let trace_id = span.trace_id;
    let span_name = span.name.clone();
    let tid = i32::try_from(span.thread_id).ok();

    rsx! {
        details { class: "{row_class}", open,
            summary {
                class: "flex cursor-pointer list-none items-center hover:bg-gray-50 focus:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-blue-600",
                aria_label: "Span {span_name}, phase {phase}, thread {span.thread_id}, total {total}, self {self_time}, cover {cover}, {child_count} nested spans, {event_count} events",
                onclick: move |_| set_trace_context(trace_id, Some(&span_name), tid),
                div { class: "w-[38%] min-w-[320px] shrink-0 px-4 py-2", style: "{indent_style}",
                    div { class: "flex min-w-0 items-center gap-2",
                        span { class: "text-xs text-gray-500 transition-transform group-open:rotate-90", aria_hidden: "true", "▶" }
                        span { class: "truncate text-xs font-medium text-gray-800", title: "{span.name}", "{span.name}" }
                        span { class: "shrink-0 rounded bg-gray-100 px-1.5 py-0.5 text-xs text-gray-500", "{phase}" }
                        span { class: "shrink-0 text-xs text-gray-500", "thread {span.thread_id} · {child_count} nested · {event_count} events" }
                        if active {
                            span { class: "shrink-0 rounded bg-amber-50 px-1.5 py-0.5 text-xs font-medium text-amber-700", "Active" }
                        }
                        if selected {
                            span { class: "shrink-0 rounded bg-blue-100 px-1.5 py-0.5 text-xs font-semibold text-blue-800", "Selected" }
                        }
                    }
                }
                SpanSummaryBar { span: span.clone(), window }
                div { class: "w-24 shrink-0 px-3 py-2 text-right font-mono text-xs text-gray-600", "{total}" }
                div { class: "w-24 shrink-0 px-3 py-2 text-right font-mono text-xs text-blue-700", "{self_time}" }
                div { class: "w-16 shrink-0 px-3 py-2 text-right font-mono text-xs text-gray-500", "{cover}" }
            }
            if location.is_some() || !events.is_empty() {
                div { class: "ml-10 border-l border-gray-200 bg-gray-50/60 px-4 py-2 text-xs text-gray-600",
                    if let Some(location) = location { p { class: "font-mono", "{location}" } }
                    for event in events {
                        p { class: "mt-1 flex gap-2",
                            span { class: "font-mono text-gray-500", "+{format_ns(event.timestamp.saturating_sub(span.start_timestamp))}" }
                            span { "{event.name}" }
                        }
                    }
                }
            }
            for child in children {
                TraceSpanRow { span: child, window, depth: depth + 1, expansion }
            }
        }
    }
}

fn filter_span_tree(spans: &[SpanInfo], query: &str) -> Vec<SpanInfo> {
    let needle = query.trim().to_ascii_lowercase();
    if needle.is_empty() {
        return spans.to_vec();
    }
    spans
        .iter()
        .filter_map(|span| {
            let mut kept = span.clone();
            kept.children = filter_span_tree(&span.children, &needle);
            let direct = span.name.to_ascii_lowercase().contains(&needle)
                || span
                    .phase
                    .as_deref()
                    .is_some_and(|value| value.to_ascii_lowercase().contains(&needle))
                || span
                    .location
                    .as_deref()
                    .is_some_and(|value| value.to_ascii_lowercase().contains(&needle));
            (direct || !kept.children.is_empty()).then_some(kept)
        })
        .collect()
}

fn count_spans(spans: &[SpanInfo]) -> usize {
    spans
        .iter()
        .map(|span| 1 + count_spans(&span.children))
        .sum()
}
fn count_events(spans: &[SpanInfo]) -> usize {
    spans
        .iter()
        .map(|span| span.events.len() + count_events(&span.children))
        .sum()
}
fn count_active(spans: &[SpanInfo]) -> usize {
    spans
        .iter()
        .map(|span| usize::from(span.end_timestamp.is_none()) + count_active(&span.children))
        .sum()
}
fn thread_count(spans: &[SpanInfo]) -> usize {
    let mut threads = HashSet::new();
    collect_threads(spans, &mut threads);
    threads.len()
}
fn collect_threads(spans: &[SpanInfo], threads: &mut HashSet<i64>) {
    for span in spans {
        threads.insert(span.thread_id);
        collect_threads(&span.children, threads);
    }
}
fn group_traces(spans: Vec<SpanInfo>) -> Vec<TraceGroup> {
    let mut groups = BTreeMap::<i64, Vec<SpanInfo>>::new();
    for span in spans {
        groups.entry(span.trace_id).or_default().push(span);
    }
    groups
        .into_iter()
        .map(|(trace_id, mut roots)| {
            roots.sort_by_key(|span| span.start_timestamp);
            TraceGroup { trace_id, roots }
        })
        .collect()
}
fn sum_self_ns(spans: &[SpanInfo]) -> i64 {
    spans
        .iter()
        .map(|span| span_self_ns(span).unwrap_or(0) + sum_self_ns(&span.children))
        .sum()
}
fn covered_root_ns(spans: &[SpanInfo], window: TraceTimeWindow) -> i64 {
    let mut intervals = spans
        .iter()
        .filter_map(|span| {
            let start = span.start_timestamp.max(window.start_ns);
            let end = span.end_timestamp?.min(window.end_ns);
            (end > start).then_some((start, end))
        })
        .collect::<Vec<_>>();
    intervals.sort_by_key(|(start, _)| *start);
    let mut covered = 0;
    let mut current: Option<(i64, i64)> = None;
    for (start, end) in intervals {
        match current {
            Some((current_start, current_end)) if start <= current_end => {
                current = Some((current_start, current_end.max(end)));
            }
            Some((current_start, current_end)) => {
                covered += current_end - current_start;
                current = Some((start, end));
            }
            None => current = Some((start, end)),
        }
    }
    if let Some((start, end)) = current {
        covered += end - start;
    }
    covered
}

fn span_total_label(span: &SpanInfo, window: TraceTimeWindow) -> String {
    if span.end_timestamp.is_none() {
        "In progress".to_string()
    } else {
        format_ns(span_total_ns(span, window))
    }
}

fn span_cover_label(span: &SpanInfo, window: TraceTimeWindow) -> String {
    if span.end_timestamp.is_none() {
        "—".to_string()
    } else {
        let cover = span_total_ns(span, window) as f64 / window.range_ns().max(1) as f64 * 100.0;
        format!("{cover:.1}%")
    }
}
fn format_ns(value: i64) -> String {
    if value >= 1_000_000_000 {
        format!("{:.2}s", value as f64 / 1_000_000_000.0)
    } else if value >= 1_000_000 {
        format!("{:.2}ms", value as f64 / 1_000_000.0)
    } else if value >= 1_000 {
        format!("{:.1}µs", value as f64 / 1_000.0)
    } else {
        format!("{value}ns")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn span(id: i64, name: &str, children: Vec<SpanInfo>) -> SpanInfo {
        SpanInfo {
            span_id: id,
            trace_id: 1,
            parent_id: None,
            name: name.to_string(),
            start_timestamp: 0,
            end_timestamp: Some(10),
            thread_id: id,
            phase: None,
            location: None,
            attributes: None,
            children,
            events: Vec::new(),
        }
    }

    fn timed_span(trace_id: i64, id: i64, start: i64, end: i64) -> SpanInfo {
        SpanInfo {
            trace_id,
            start_timestamp: start,
            end_timestamp: Some(end),
            ..span(id, "span", vec![])
        }
    }

    #[test]
    fn search_keeps_matching_ancestor_path() {
        let filtered = filter_span_tree(
            &[span(1, "root", vec![span(2, "target", vec![])])],
            "target",
        );
        assert_eq!(filtered[0].children[0].name, "target");
    }

    #[test]
    fn pinned_training_coordinates_filter_trace_tree_without_flattening_it() {
        let mut matching = span(2, "train.step", vec![span(3, "forward", vec![])]);
        matching.thread_id = 17;
        matching.attributes = Some(r#"{"rank":"7","local_step":31}"#.to_string());
        let root = span(1, "root", vec![matching]);
        let context = InvestigationContext {
            tid: Some(17),
            rank: Some(7),
            span_name: Some("train.step".to_string()),
            local_step: Some(31),
            ..Default::default()
        };

        let filtered = filter_span_context_tree(&[root], &context);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].children[0].name, "train.step");
        assert_eq!(filtered[0].children[0].children[0].name, "forward");
    }

    #[test]
    fn trace_groups_keep_roots_with_their_trace() {
        let groups = group_traces(vec![
            timed_span(7, 1, 20, 30),
            timed_span(3, 2, 10, 40),
            timed_span(7, 3, 5, 15),
        ]);

        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].trace_id, 3);
        assert_eq!(groups[1].trace_id, 7);
        assert_eq!(groups[1].roots[0].span_id, 3);
    }

    #[test]
    fn trace_group_coverage_uses_interval_union() {
        let window = TraceTimeWindow {
            start_ns: 0,
            end_ns: 100,
        };
        let roots = vec![timed_span(1, 1, 10, 50), timed_span(1, 2, 40, 80)];
        let summary = TraceGroupSummary::from_roots(&roots, window);

        assert_eq!(summary.covered_ns, 70);
        assert_eq!(summary.window_ns(), 70);
        assert!((summary.cover_pct(window) - 70.0).abs() < f64::EPSILON);
    }

    #[test]
    fn active_spans_do_not_claim_completed_duration_or_coverage() {
        let window = TraceTimeWindow {
            start_ns: 0,
            end_ns: 1_000,
        };
        let active = SpanInfo {
            end_timestamp: None,
            ..timed_span(1, 1, 100, 200)
        };
        assert_eq!(span_total_label(&active, window), "In progress");
        assert_eq!(span_cover_label(&active, window), "—");
        assert_eq!(covered_root_ns(&[active], window), 0);
    }
}
