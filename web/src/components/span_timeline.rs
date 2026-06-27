//! Vertical timeline lane paired with the span tree on the Spans page.
//!
//! Each visible tree row gets a timeline row on the left; expanding the tree
//! grows the timeline stack so hierarchy and timing stay aligned.

use dioxus::prelude::*;

use crate::api::SpanInfo;

const TIMELINE_LANE_PX: f64 = 148.0;
const MIN_BAR_PX: f64 = 3.0;

/// Nanosecond window covering all spans in the current tree.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TraceTimeWindow {
    pub start_ns: i64,
    pub end_ns: i64,
}

impl TraceTimeWindow {
    pub fn from_spans(spans: &[SpanInfo]) -> Self {
        let mut start = i64::MAX;
        let mut end = i64::MIN;

        fn walk(span: &SpanInfo, start: &mut i64, end: &mut i64) {
            *start = (*start).min(span.start_timestamp);
            let span_end = span.end_timestamp.unwrap_or(span.start_timestamp);
            *end = (*end).max(span_end);
            for child in &span.children {
                walk(child, start, end);
            }
        }

        for span in spans {
            walk(span, &mut start, &mut end);
        }

        if start == i64::MAX {
            return Self {
                start_ns: 0,
                end_ns: 1,
            };
        }
        if end <= start {
            end = start + 1;
        }
        Self {
            start_ns: start,
            end_ns: end,
        }
    }

    pub fn range_ns(&self) -> i64 {
        (self.end_ns - self.start_ns).max(1)
    }

    pub fn offset_px(&self, timestamp_ns: i64) -> f64 {
        let pct = (timestamp_ns - self.start_ns) as f64 / self.range_ns() as f64;
        (pct.clamp(0.0, 1.0) * TIMELINE_LANE_PX).max(0.0)
    }

    pub fn width_px(&self, start_ns: i64, end_ns: Option<i64>) -> f64 {
        let end = end_ns.unwrap_or(self.end_ns);
        let dur = (end - start_ns).max(0) as f64;
        let px = dur / self.range_ns() as f64 * TIMELINE_LANE_PX;
        px.max(MIN_BAR_PX)
    }
}

pub fn format_axis_label(duration_ns: f64) -> String {
    if duration_ns >= 1_000_000_000.0 {
        format!("{:.2}s", duration_ns / 1_000_000_000.0)
    } else if duration_ns >= 1_000_000.0 {
        format!("{:.1}ms", duration_ns / 1_000_000.0)
    } else if duration_ns >= 1_000.0 {
        format!("{:.0}µs", duration_ns / 1_000.0)
    } else {
        format!("{:.0}ns", duration_ns)
    }
}

fn span_bar_style(phase: Option<&str>, active: bool) -> (&'static str, &'static str) {
    if active {
        return ("bg-amber-200/80", "bg-amber-500");
    }
    match phase {
        Some("forward") => ("bg-blue-200/70", "bg-blue-500"),
        Some("backward") => ("bg-purple-200/70", "bg-purple-500"),
        Some("optimizer") | Some("step") => ("bg-amber-200/70", "bg-amber-500"),
        Some("idle") => ("bg-gray-200/70", "bg-gray-400"),
        _ => ("bg-emerald-200/70", "bg-emerald-500"),
    }
}

fn span_tooltip(span: &SpanInfo, window: TraceTimeWindow) -> String {
    let start = format_axis_label((span.start_timestamp - window.start_ns) as f64);
    let end = span
        .end_timestamp
        .map(|t| format_axis_label((t - window.start_ns) as f64))
        .unwrap_or_else(|| "active".to_string());
    let dur = span
        .end_timestamp
        .map(|t| format_axis_label((t - span.start_timestamp) as f64))
        .unwrap_or_else(|| "active".to_string());
    format!(
        "{}\nphase: {}\noffset: {} · end: {}\nduration: {}",
        span.name,
        span.phase.as_deref().unwrap_or("—"),
        start,
        end,
        dur,
    )
}

#[component]
pub fn SpanTimelineHeader(window: TraceTimeWindow) -> Element {
    let total = format_axis_label(window.range_ns() as f64);
    let mid = format_axis_label(window.range_ns() as f64 / 2.0);
    rsx! {
        div {
            class: "flex shrink-0 border-b border-gray-200 bg-gray-50/90 sticky top-0 z-10",
            div {
                class: "shrink-0 border-r border-gray-200 px-2 py-1.5",
                style: "width: {TIMELINE_LANE_PX}px",
                div { class: "text-[10px] font-semibold uppercase tracking-wide text-gray-500 mb-1",
                    "Timeline"
                }
                div { class: "relative h-4 text-[9px] text-gray-400 font-mono tabular-nums",
                    span { class: "absolute left-0 top-0", "0" }
                    span { class: "absolute left-1/2 -translate-x-1/2 top-0", "{mid}" }
                    span { class: "absolute right-0 top-0", "{total}" }
                    div { class: "absolute inset-x-0 top-[14px] h-px bg-gray-200" }
                    div { class: "absolute left-0 top-[11px] w-px h-[7px] bg-gray-300" }
                    div { class: "absolute left-1/2 top-[11px] w-px h-[7px] bg-gray-300" }
                    div { class: "absolute right-0 top-[11px] w-px h-[7px] bg-gray-300" }
                }
            }
            div { class: "flex-1 px-3 py-1.5 text-[10px] font-semibold uppercase tracking-wide text-gray-500",
                "Span tree"
            }
        }
    }
}

#[component]
pub fn SpanTimelineLegend() -> Element {
    rsx! {
        div {
            class: "flex flex-wrap items-center gap-x-4 gap-y-1 px-2 py-1.5 border-b border-gray-100 bg-white text-[10px] text-gray-500 sticky top-[52px] z-10",
            span { class: "font-medium text-gray-600", "Lane" }
            div { class: "inline-flex items-center gap-1",
                span { class: "w-3 h-2 rounded-sm bg-blue-500" }
                span { "forward" }
            }
            div { class: "inline-flex items-center gap-1",
                span { class: "w-3 h-2 rounded-sm bg-purple-500" }
                span { "backward" }
            }
            div { class: "inline-flex items-center gap-1",
                span { class: "w-3 h-2 rounded-sm bg-amber-500" }
                span { "optimizer" }
            }
            div { class: "inline-flex items-center gap-1",
                span { class: "w-3 h-2 rounded-sm bg-emerald-500" }
                span { "other" }
            }
            div { class: "inline-flex items-center gap-1",
                span { class: "w-3 h-2 rounded-sm bg-amber-500 animate-pulse" }
                span { "active" }
            }
        }
    }
}

#[component]
pub fn SpanTimelineBar(span: SpanInfo, window: TraceTimeWindow, depth: usize) -> Element {
    let active = span.end_timestamp.is_none();
    let left = window.offset_px(span.start_timestamp);
    let width = window.width_px(span.start_timestamp, span.end_timestamp);
    let (track_bg, bar_bg) = span_bar_style(span.phase.as_deref(), active);
    let indent = depth * 10;
    let tooltip = span_tooltip(&span, window);
    let lane_inner = TIMELINE_LANE_PX - indent as f64;
    let bar_left = left.min(lane_inner - MIN_BAR_PX).max(0.0);
    let bar_width = width.min(lane_inner - bar_left);
    let guide_left = indent.saturating_sub(6);

    rsx! {
        div {
            class: "shrink-0 border-r border-gray-100 flex items-center py-0.5 relative",
            style: "width: {TIMELINE_LANE_PX}px; padding-left: {indent}px",
            title: "{tooltip}",
            if depth > 0 {
                div {
                    class: "absolute top-0 bottom-1/2 w-px bg-gray-200",
                    style: "left: {guide_left}px",
                }
                div {
                    class: "absolute top-1/2 w-2 h-px bg-gray-200",
                    style: "left: {guide_left}px",
                }
            }
            div { class: "relative h-[22px] flex-1 min-w-0 pr-1",
                div { class: "absolute inset-y-[7px] inset-x-0 rounded-full {track_bg}" }
                div {
                    class: "absolute top-[5px] h-[12px] rounded-sm {bar_bg} shadow-sm",
                    style: "left: {bar_left:.2}px; width: {bar_width:.2}px;",
                }
                div {
                    class: "absolute top-[9px] w-1.5 h-1.5 rounded-full {bar_bg} ring-2 ring-white -translate-x-1/2",
                    style: "left: {bar_left:.2}px;",
                }
                if active {
                    div {
                        class: "absolute top-[10px] h-0.5 bg-amber-400/60 animate-pulse",
                        style: "left: calc({bar_left:.2}px + {bar_width:.2}px); right: 0;",
                    }
                }
            }
        }
    }
}

/// Empty lane cell for detail rows (attributes / events) that have no bar.
#[component]
pub fn SpanTimelineSpacer() -> Element {
    rsx! {
        div {
            class: "shrink-0 border-r border-gray-100 bg-gray-50/30",
            style: "width: {TIMELINE_LANE_PX}px",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(start: i64, end: Option<i64>) -> SpanInfo {
        SpanInfo {
            span_id: 1,
            trace_id: 1,
            parent_id: None,
            name: "test".into(),
            start_timestamp: start,
            end_timestamp: end,
            thread_id: 0,
            phase: None,
            location: None,
            attributes: None,
            children: vec![],
            events: vec![],
        }
    }

    #[test]
    fn window_from_spans_uses_min_max() {
        let roots = vec![
            span(100, Some(500)),
            SpanInfo {
                children: vec![span(200, Some(800))],
                ..span(50, Some(300))
            },
        ];
        let w = TraceTimeWindow::from_spans(&roots);
        assert_eq!(w.start_ns, 50);
        assert_eq!(w.end_ns, 800);
    }

    #[test]
    fn offset_and_width_px() {
        let w = TraceTimeWindow {
            start_ns: 0,
            end_ns: 1000,
        };
        assert!((w.offset_px(500) - TIMELINE_LANE_PX / 2.0).abs() < 0.01);
        assert!((w.width_px(0, Some(1000)) - TIMELINE_LANE_PX).abs() < 0.01);
        assert!(w.width_px(0, Some(1)) >= MIN_BAR_PX);
    }
}
