//! Timeline primitives for the Spans page: summary tree and compact lane view.

use dioxus::prelude::*;

use crate::api::SpanInfo;

#[cfg(test)]
const TIMELINE_LANE_PX: f64 = 148.0;
#[cfg(test)]
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

    #[cfg(test)]
    pub fn offset_px(&self, timestamp_ns: i64) -> f64 {
        let pct = (timestamp_ns - self.start_ns) as f64 / self.range_ns() as f64;
        (pct.clamp(0.0, 1.0) * TIMELINE_LANE_PX).max(0.0)
    }

    #[cfg(test)]
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

/// Parent time not covered by the union of its direct child intervals.
pub fn span_self_ns(span: &SpanInfo) -> Option<i64> {
    let end = span.end_timestamp?;
    let mut intervals = span
        .children
        .iter()
        .filter_map(|child| {
            let child_end = child.end_timestamp?;
            let start = child.start_timestamp.max(span.start_timestamp);
            let end = child_end.min(end);
            (end > start).then_some((start, end))
        })
        .collect::<Vec<_>>();
    intervals.sort_by_key(|(start, _)| *start);

    let mut covered = 0i64;
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
    Some((end - span.start_timestamp - covered).max(0))
}

/// Inclusive span duration inside the currently visible window.
pub fn span_total_ns(span: &SpanInfo, window: TraceTimeWindow) -> i64 {
    let end = span.end_timestamp.unwrap_or(window.end_ns);
    (end - span.start_timestamp).max(0)
}

/// Percentage of the visible trace window covered by this span.
#[cfg(test)]
pub fn span_cover_pct(span: &SpanInfo, window: TraceTimeWindow) -> f64 {
    span_total_ns(span, window) as f64 / window.range_ns() as f64 * 100.0
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
pub fn SpanSummaryHeader(window: TraceTimeWindow) -> Element {
    let total = format_axis_label(window.range_ns() as f64);
    let quarter = format_axis_label(window.range_ns() as f64 / 4.0);
    let middle = format_axis_label(window.range_ns() as f64 / 2.0);
    let three_quarters = format_axis_label(window.range_ns() as f64 * 3.0 / 4.0);
    rsx! {
        div { class: "sticky top-0 z-10 flex min-w-[960px] border-b border-gray-200 bg-gray-50/95 text-xs font-semibold uppercase tracking-wide text-gray-500 shadow-sm",
            div { class: "w-[38%] min-w-[320px] shrink-0 px-4 py-2", "Structure" }
            div { class: "min-w-[260px] flex-1 px-3 py-1.5",
                div { class: "flex items-center justify-between",
                    span { "Position / occupancy" }
                    span { class: "font-mono font-normal normal-case text-gray-500", "{total} window" }
                }
                div { class: "relative mt-1 h-4 border-t border-gray-300 font-mono text-[11px] font-normal normal-case text-gray-500",
                    span { class: "absolute left-0 top-0", "0" }
                    span { class: "absolute left-1/4 top-0 -translate-x-1/2", "{quarter}" }
                    span { class: "absolute left-1/2 top-0 -translate-x-1/2", "{middle}" }
                    span { class: "absolute left-3/4 top-0 -translate-x-1/2", "{three_quarters}" }
                    span { class: "absolute right-0 top-0", "{total}" }
                    for position in [0_u8, 25, 50, 75, 100] {
                        div {
                            class: "absolute -top-px h-1.5 w-px bg-gray-300",
                            style: "left: {position}%",
                        }
                    }
                }
            }
            div { class: "w-24 shrink-0 px-3 py-2 text-right", "Total" }
            div { class: "w-24 shrink-0 px-3 py-2 text-right", "Self" }
            div { class: "w-16 shrink-0 px-3 py-2 text-right", "Cover" }
        }
    }
}

#[component]
pub fn SpanSummaryBar(span: SpanInfo, window: TraceTimeWindow) -> Element {
    let active = span.end_timestamp.is_none();
    let (track_bg, bar_bg) = span_bar_style(span.phase.as_deref(), active);
    let left = (span.start_timestamp - window.start_ns) as f64 / window.range_ns() as f64 * 100.0;
    let width = span_total_ns(&span, window) as f64 / window.range_ns() as f64 * 100.0;
    let tooltip = span_tooltip(&span, window);
    rsx! {
        div { class: "min-w-[260px] flex-1 px-3 py-2", role: "img", aria_label: "{tooltip}", title: "{tooltip}",
            div { class: "relative h-4 overflow-hidden rounded-sm border border-gray-200 bg-gray-50",
                div {
                    class: "absolute inset-y-[3px] rounded-sm {track_bg}",
                    style: "left: {left:.4}%; width: max({width:.4}%, 3px);",
                }
                div {
                    class: "absolute inset-y-[5px] rounded-sm {bar_bg}",
                    style: "left: {left:.4}%; width: max({width:.4}%, 3px);",
                }
                for child in span.children.iter() {
                    {
                        let child_left = (child.start_timestamp - window.start_ns) as f64
                            / window.range_ns() as f64
                            * 100.0;
                        let child_width = span_total_ns(child, window) as f64
                            / window.range_ns() as f64
                            * 100.0;
                        rsx! {
                            div {
                                class: "absolute inset-y-[2px] min-w-[2px] rounded-sm bg-gray-900/45 ring-1 ring-white/70",
                                style: "left: {child_left:.4}%; width: max({child_width:.4}%, 2px);",
                                title: "child: {child.name}",
                            }
                        }
                    }
                }
                if active {
                    div {
                        class: "absolute inset-y-0 w-px animate-pulse bg-amber-600 motion-reduce:animate-none",
                        style: "left: calc({left:.4}% + max({width:.4}%, 3px));",
                    }
                }
            }
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

    #[test]
    fn self_time_unions_overlapping_children() {
        let parent = SpanInfo {
            children: vec![span(10, Some(50)), span(30, Some(70))],
            ..span(0, Some(100))
        };
        assert_eq!(span_self_ns(&parent), Some(40));
    }

    #[test]
    fn total_and_cover_use_visible_window_for_active_spans() {
        let window = TraceTimeWindow {
            start_ns: 0,
            end_ns: 1000,
        };
        let active = span(250, None);

        assert_eq!(span_total_ns(&active, window), 750);
        assert!((span_cover_pct(&active, window) - 75.0).abs() < 0.01);
    }
}
