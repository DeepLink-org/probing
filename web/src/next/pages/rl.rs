use std::collections::HashSet;

use dioxus::prelude::*;

use crate::api::{ApiClient, SpanInfo};
use crate::components::span_timeline::{
    span_self_ns, span_total_ns, SpanSummaryBar, SpanSummaryHeader, TraceTimeWindow,
};
use crate::state::rl::{RL_EVENT_LIMIT, ROLLOUT_FILTER};
use crate::utils::tracing_viewer;

use super::super::components::{
    ActionButton, EvidenceMetric, LoadingPanel, SectionCard, UnavailablePanel, WorkspacePage,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RlMode {
    Rollout,
    Train,
    Spans,
    ProcessTimeline,
    Perfetto,
}

impl RlMode {
    fn title(self) -> &'static str {
        match self {
            Self::Rollout => "RL Rollout",
            Self::Train => "Policy Training",
            Self::Spans => "RL Spans",
            Self::ProcessTimeline => "Process Timeline",
            Self::Perfetto => "RL Perfetto Export",
        }
    }

    fn subtitle(self) -> &'static str {
        match self {
            Self::Rollout => "Reported rollout spans, optionally restricted by rollout ID.",
            Self::Train => "Training-related spans and their nested phase timing.",
            Self::Spans => "Cross-process span trees at the requested event limit.",
            Self::ProcessTimeline => {
                "The same span set grouped by its reported process and thread fields."
            }
            Self::Perfetto => "Inspect the loaded hierarchy or export it as a Chrome trace.",
        }
    }
}

#[component]
pub fn RolloutPage() -> Element {
    rsx! { RlEvidencePage { mode: RlMode::Rollout } }
}

#[component]
pub fn RlTrainPage() -> Element {
    rsx! { RlEvidencePage { mode: RlMode::Train } }
}

#[component]
pub fn RlSpansPage() -> Element {
    rsx! { RlEvidencePage { mode: RlMode::Spans } }
}

#[component]
pub fn ProcessTimelinePage() -> Element {
    rsx! { RlEvidencePage { mode: RlMode::ProcessTimeline } }
}

#[component]
pub fn PerfettoPage() -> Element {
    rsx! { RlEvidencePage { mode: RlMode::Perfetto } }
}

#[derive(Clone, Debug, PartialEq)]
struct RlEvidence {
    spans: Vec<SpanInfo>,
    processes_queried: usize,
    processes_failed: Vec<i32>,
}

#[component]
fn RlEvidencePage(mode: RlMode) -> Element {
    let request = use_memo(move || {
        let limit = if mode == RlMode::Perfetto {
            2_000
        } else {
            *RL_EVENT_LIMIT.read()
        };
        let rollout = if matches!(mode, RlMode::Rollout | RlMode::Perfetto) {
            ROLLOUT_FILTER.read().trim().to_string()
        } else {
            String::new()
        };
        (limit, rollout)
    });
    let evidence = use_resource(move || {
        let (limit, rollout) = request();
        async move { load_rl_evidence(limit, rollout).await }
    });
    let state = evidence.read().clone();

    rsx! {
        WorkspacePage {
            title: mode.title().to_string(),
            subtitle: mode.subtitle().to_string(),
            match state {
                None => rsx! { LoadingPanel { label: "Loading RL span evidence".to_string() } },
                Some(Err(error)) => rsx! { UnavailablePanel {
                    label: "RL span evidence unavailable".to_string(),
                    detail: error.display_message(),
                }},
                Some(Ok(evidence)) => rsx! { RlEvidenceView { mode, evidence } },
            }
        }
    }
}

async fn load_rl_evidence(
    limit: usize,
    rollout: String,
) -> crate::utils::error::Result<RlEvidence> {
    let client = ApiClient::new();
    let processes = client.get_trace_processes().await.unwrap_or_default();
    let mut spans = if rollout.is_empty() {
        client.get_span_tree(Some(limit)).await?
    } else {
        client.get_span_tree_for_rollout_id(&rollout).await?
    };
    let mut failed = Vec::new();
    for process in &processes {
        let remote = if rollout.is_empty() {
            client.get_span_tree_for_pid(process.pid, Some(limit)).await
        } else {
            client
                .get_span_tree_for_pid_and_rollout_id(process.pid, &rollout)
                .await
        };
        match remote {
            Ok(mut process_spans) => spans.append(&mut process_spans),
            Err(_) => failed.push(process.pid),
        }
    }
    dedupe_and_sort_spans(&mut spans);
    Ok(RlEvidence {
        spans,
        processes_queried: processes.len() + 1,
        processes_failed: failed,
    })
}

fn dedupe_and_sort_spans(spans: &mut Vec<SpanInfo>) {
    let mut seen = HashSet::new();
    spans.retain(|span| seen.insert((span.span_id, span.thread_id, span.start_timestamp)));
    spans.sort_by_key(|span| span.start_timestamp);
}

#[component]
fn RlEvidenceView(mode: RlMode, evidence: RlEvidence) -> Element {
    let total_spans = count_spans(&evidence.spans);
    let total_events = count_events(&evidence.spans);
    let window = TraceTimeWindow::from_spans(&evidence.spans);
    let duration = format_ns(window.range_ns());
    let export_spans = evidence.spans.clone();
    let failed_processes = evidence
        .processes_failed
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ");

    rsx! {
        SectionCard {
            title: "Loaded evidence".to_string(),
            subtitle: Some("Counts describe the current in-browser span set without classifying it.".to_string()),
            actions: if mode == RlMode::Perfetto && !export_spans.is_empty() {
                Some(rsx! {
                    ActionButton {
                        label: "Open in Perfetto ↗".to_string(),
                        onclick: move |_| {
                            let json = spans_to_chrome_trace(&export_spans);
                            if let Err(error) = tracing_viewer::open_perfetto_window(&json) {
                                log::warn!("Perfetto export failed: {error}");
                            }
                        },
                    }
                })
            } else { None },
            div { class: "grid grid-cols-4 divide-x divide-gray-200",
                EvidenceMetric { label: "Processes queried", value: evidence.processes_queried.to_string() }
                EvidenceMetric { label: "Root / all spans", value: format!("{} / {total_spans}", evidence.spans.len()) }
                EvidenceMetric { label: "Events", value: total_events.to_string() }
                EvidenceMetric { label: "Time window", value: duration }
            }
            if !evidence.processes_failed.is_empty() {
                div { class: "mt-3 rounded-md border border-amber-200 bg-amber-50 px-3 py-2 text-xs text-amber-900",
                    "No span response from PID(s): {failed_processes}"
                }
            }
        }

        SectionCard {
            title: if mode == RlMode::ProcessTimeline { "Process span hierarchy".to_string() } else { "Span hierarchy".to_string() },
            subtitle: Some("Expand only the branches required; collapsed rows retain duration, self time, coverage, child count, and event count.".to_string()),
            body_class: "p-0".to_string(),
            if evidence.spans.is_empty() {
                div { class: "p-4", UnavailablePanel {
                    label: "No RL spans returned".to_string(),
                    detail: "Change the rollout filter or wait for trace_event samples.".to_string(),
                }}
            } else {
                div { class: "overflow-x-auto",
                    SpanSummaryHeader { window }
                    for span in evidence.spans {
                        RlSpanRow { span, window, depth: 0 }
                    }
                }
            }
        }
    }
}

#[component]
fn RlSpanRow(span: SpanInfo, window: TraceTimeWindow, depth: usize) -> Element {
    let child_count = span.children.len();
    let event_count = span.events.len();
    let total = format_ns(span_total_ns(&span, window));
    let self_time = span_self_ns(&span)
        .map(format_ns)
        .unwrap_or_else(|| "—".to_string());
    let cover = span_total_ns(&span, window) as f64 / window.range_ns() as f64 * 100.0;
    let children = span.children.clone();
    let indent = depth * 14;
    let indent_style = format!("padding-left: {}px", 16 + indent);

    rsx! {
        details { class: "group min-w-[960px] border-b border-gray-100", open: depth == 0,
            summary { class: "flex cursor-pointer list-none items-center hover:bg-gray-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-blue-600",
                div { class: "w-[38%] min-w-[320px] shrink-0 px-4 py-2", style: "{indent_style}",
                    div { class: "flex min-w-0 items-center gap-2",
                        span { class: "text-xs text-gray-500 transition-transform group-open:rotate-90", "▶" }
                        span { class: "break-all text-xs font-medium text-gray-800", "{span.name}" }
                        span { class: "shrink-0 text-xs text-gray-500", "{child_count} nested · {event_count} events" }
                    }
                }
                SpanSummaryBar { span: span.clone(), window }
                div { class: "w-24 shrink-0 px-3 py-2 text-right font-mono text-xs text-gray-600", "{total}" }
                div { class: "w-24 shrink-0 px-3 py-2 text-right font-mono text-xs text-blue-700", "{self_time}" }
                div { class: "w-16 shrink-0 px-3 py-2 text-right font-mono text-xs text-gray-500", "{cover:.1}%" }
            }
            for child in children {
                RlSpanRow { span: child, window, depth: depth + 1 }
            }
        }
    }
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

fn spans_to_chrome_trace(spans: &[SpanInfo]) -> String {
    fn append(span: &SpanInfo, events: &mut Vec<serde_json::Value>) {
        let duration = span
            .end_timestamp
            .unwrap_or(span.start_timestamp)
            .saturating_sub(span.start_timestamp);
        events.push(serde_json::json!({
            "name": span.name,
            "cat": span.phase.as_deref().unwrap_or("rl"),
            "ph": "X",
            "ts": span.start_timestamp as f64 / 1_000.0,
            "dur": duration as f64 / 1_000.0,
            "pid": span.trace_id,
            "tid": span.thread_id,
            "args": {"span_id": span.span_id, "location": span.location},
        }));
        for child in &span.children {
            append(child, events);
        }
    }
    let mut events = Vec::new();
    for span in spans {
        append(span, &mut events);
    }
    serde_json::json!({"traceEvents": events}).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(id: i64, children: Vec<SpanInfo>) -> SpanInfo {
        SpanInfo {
            span_id: id,
            trace_id: 1,
            parent_id: None,
            name: format!("span-{id}"),
            start_timestamp: id * 1_000,
            end_timestamp: Some(id * 1_000 + 500),
            thread_id: 3,
            phase: None,
            location: None,
            attributes: None,
            children,
            events: Vec::new(),
        }
    }

    #[test]
    fn collapsed_summary_count_keeps_nested_spans() {
        let spans = vec![span(1, vec![span(2, vec![span(3, vec![])])])];
        assert_eq!(count_spans(&spans), 3);
        assert!(spans_to_chrome_trace(&spans).contains("span-3"));
    }
}
