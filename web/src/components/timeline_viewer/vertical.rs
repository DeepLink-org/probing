use dioxus::prelude::*;

use crate::components::common::EmptyState;
use crate::components::icon::Icon;

use super::model::{count_slices_in_tracks, TimelineModel, TimelineSlice, TimelineTrack};
use super::viewer::{filter_tracks, format_duration_us, slice_color};

#[derive(Clone, Debug, PartialEq)]
struct SliceGroup {
    name: String,
    cat: String,
    slices: Vec<TimelineSlice>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct SliceSummary {
    occurrences: usize,
    descendants: usize,
    total_us: f64,
    self_us: f64,
    covered_us: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ExpansionCommand {
    revision: u32,
    expanded: bool,
}

#[component]
pub(super) fn VerticalTimelineView(
    model: TimelineModel,
    filter: Signal<String>,
    vertical: Signal<bool>,
    on_export: EventHandler<()>,
) -> Element {
    let mut expansion = use_signal(ExpansionCommand::default);
    let tracks = filter_tracks(&model, &filter());
    let shown = count_slices_in_tracks(&tracks);
    let window = format_duration_us(model.range_us());

    rsx! {
        div { class: "flex h-full min-h-[600px] flex-col",
            div { class: "flex flex-wrap items-center gap-2 border-b border-gray-200 bg-gray-50/80 px-4 py-2.5",
                div { class: "relative min-w-[180px] max-w-sm flex-1",
                    Icon { icon: &icondata::AiSearchOutlined, class: "pointer-events-none absolute left-2 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-gray-400" }
                    input {
                        r#type: "search",
                        class: "w-full rounded-md border border-gray-300 bg-white py-1.5 pl-7 pr-2 text-xs focus:border-blue-500 focus:outline-none focus:ring-2 focus:ring-blue-500/20",
                        placeholder: "Find slice or category",
                        value: "{filter}",
                        oninput: move |event| filter.set(event.value()),
                    }
                }
                button {
                    class: toolbar_button(),
                    onclick: move |_| expansion.set(ExpansionCommand { revision: expansion().revision.saturating_add(1), expanded: true }),
                    "Expand all"
                }
                button {
                    class: toolbar_button(),
                    onclick: move |_| expansion.set(ExpansionCommand { revision: expansion().revision.saturating_add(1), expanded: false }),
                    "Collapse all"
                }
                div { class: "ml-auto flex items-center rounded-md border border-gray-300 bg-white p-0.5 text-[10px]",
                    button { class: "rounded bg-blue-600 px-2 py-1 font-medium text-white", "Tree" }
                    button { class: "rounded px-2 py-1 text-gray-500 hover:bg-gray-50", onclick: move |_| vertical.set(false), "Timeline" }
                }
                button { class: toolbar_button(), onclick: move |_| on_export.call(()), "Perfetto ↗" }
            }
            div { class: "flex flex-wrap items-center gap-x-4 gap-y-1 border-b border-gray-100 bg-white px-4 py-2 text-[10px] text-gray-500",
                span { class: "font-medium text-gray-700", "{window} window" }
                span { "{tracks.len()} tracks" }
                span { "{shown} / {model.event_count} slices" }
                span { "bars = position in parent · total = inclusive · self = outside children" }
            }
            if tracks.is_empty() {
                div { class: "flex-1 p-8",
                    EmptyState { message: format!("No slices match \"{}\"", filter()) }
                }
            } else {
                div { class: "flex-1 overflow-auto bg-gray-50/50 p-3",
                    div { class: "mx-auto max-w-7xl space-y-3",
                        for track in tracks {
                            TimelineTrackTree {
                                key: "{track.pid}-{track.tid}",
                                track,
                                trace_start_us: model.min_ts_us,
                                trace_range_us: model.range_us(),
                                expansion,
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn TimelineTrackTree(
    track: TimelineTrack,
    trace_start_us: f64,
    trace_range_us: f64,
    expansion: Signal<ExpansionCommand>,
) -> Element {
    let mut expanded = use_signal(|| true);
    let slice_count = count_slices_in_tracks(std::slice::from_ref(&track));
    let groups = group_slices(&track.slices);
    let summary = summarize_slices(&track.slices, trace_start_us, trace_range_us);
    let intervals = intervals_for(&track.slices);

    sync_expansion(expanded, expansion);

    rsx! {
        section { class: "overflow-hidden rounded-lg border border-gray-200 bg-white",
            div { class: "flex min-w-0 items-center gap-2 border-b border-gray-100 bg-gray-50/80 px-3 py-2",
                button {
                    class: "flex h-5 w-5 shrink-0 items-center justify-center text-gray-500 hover:text-gray-800",
                    aria_label: if expanded() { "Collapse track" } else { "Expand track" },
                    onclick: move |_| expanded.set(!expanded()),
                    Icon { icon: if expanded() { &icondata::AiCaretDownOutlined } else { &icondata::AiCaretRightOutlined }, class: "h-3 w-3" }
                }
                div { class: "min-w-0 w-64 shrink-0",
                    h3 { class: "truncate text-xs font-semibold text-gray-900", title: "{track.label}", "{track.label}" }
                    p { class: "font-mono text-[9px] text-gray-400", "pid {track.pid} · tid {track.tid}" }
                }
                TimeStrip {
                    intervals,
                    window_start_us: trace_start_us,
                    window_range_us: trace_range_us,
                    color: "bg-slate-500",
                    label: "Track activity across the trace window",
                }
                span { class: "w-20 shrink-0 text-right font-mono text-[10px] tabular-nums text-gray-600", "{format_duration_us(summary.total_us)}" }
                span { class: "w-20 shrink-0 text-right font-mono text-[10px] tabular-nums text-blue-700", "{format_duration_us(summary.self_us)}" }
                span { class: "w-14 shrink-0 text-right text-[10px] tabular-nums text-gray-500", "{coverage_percent(summary, trace_range_us):.1}%" }
            }
            div { class: "flex items-center gap-2 border-b border-gray-100 bg-white px-3 py-1.5 text-[9px] font-medium uppercase tracking-wide text-gray-400",
                span { class: "w-5 shrink-0" }
                span { class: "w-64 shrink-0", "Structure · {groups.len()} groups · {slice_count} slices" }
                span { class: "min-w-[160px] flex-1", "Position / occupancy" }
                span { class: "w-20 shrink-0 text-right", "Total" }
                span { class: "w-20 shrink-0 text-right", "Self" }
                span { class: "w-14 shrink-0 text-right", "Cover" }
            }
            if expanded() {
                div { class: "py-1",
                    for group in groups {
                        SliceGroupNode {
                            key: "{group.cat}-{group.name}-{group.slices[0].start_us}",
                            group,
                            depth: 0,
                            window_start_us: trace_start_us,
                            window_range_us: trace_range_us,
                            trace_start_us,
                            expansion,
                        }
                    }
                }
            } else {
                div { class: "px-10 py-2 text-[10px] text-gray-400", "{groups.len()} groups collapsed · {slice_count} slices remain summarized above" }
            }
        }
    }
}

#[component]
fn SliceGroupNode(
    group: SliceGroup,
    depth: usize,
    window_start_us: f64,
    window_range_us: f64,
    trace_start_us: f64,
    expansion: Signal<ExpansionCommand>,
) -> Element {
    if group.slices.len() == 1 {
        return rsx! {
            TimelineSliceNode {
                slice: group.slices[0].clone(),
                depth,
                occurrence: None,
                window_start_us,
                window_range_us,
                trace_start_us,
                expansion,
            }
        };
    }

    rsx! {
        RepeatedSliceGroupNode {
            group,
            depth,
            window_start_us,
            window_range_us,
            trace_start_us,
            expansion,
        }
    }
}

#[component]
fn RepeatedSliceGroupNode(
    group: SliceGroup,
    depth: usize,
    window_start_us: f64,
    window_range_us: f64,
    trace_start_us: f64,
    expansion: Signal<ExpansionCommand>,
) -> Element {
    let mut expanded = use_signal(|| false);
    let summary = summarize_slices(&group.slices, window_start_us, window_range_us);
    let intervals = intervals_for(&group.slices);
    let color = slice_color(&group.cat);
    let average = summary.total_us / summary.occurrences.max(1) as f64;
    let indent = depth * 14;

    sync_expansion(expanded, expansion);

    rsx! {
        div { class: "border-b border-gray-100 last:border-b-0",
            div { class: "flex min-w-0 items-center gap-2 px-3 py-1.5 hover:bg-gray-50", style: "padding-left: {12 + indent}px",
                button {
                    class: "flex h-5 w-5 shrink-0 items-center justify-center text-gray-400 hover:text-gray-700",
                    aria_label: if expanded() { "Collapse repeated slices" } else { "Expand repeated slices" },
                    onclick: move |_| expanded.set(!expanded()),
                    Icon { icon: if expanded() { &icondata::AiCaretDownOutlined } else { &icondata::AiCaretRightOutlined }, class: "h-3 w-3" }
                }
                div { class: "flex min-w-0 w-64 shrink-0 items-center gap-2",
                    span { class: "h-2.5 w-2.5 shrink-0 rounded-sm {color}" }
                    span { class: "min-w-0 truncate text-xs font-semibold text-gray-900", title: "{group.name}", "{group.name}" }
                    span { class: "rounded bg-gray-100 px-1.5 py-0.5 font-mono text-[9px] text-gray-600", "×{summary.occurrences}" }
                    if summary.descendants > summary.occurrences {
                        span { class: "text-[9px] text-gray-400", "+{summary.descendants - summary.occurrences} nested" }
                    }
                }
                TimeStrip {
                    intervals,
                    window_start_us,
                    window_range_us,
                    color,
                    label: "Repeated slice positions in the current window",
                }
                span { class: "w-20 shrink-0 text-right font-mono text-[10px] tabular-nums text-gray-600", title: "avg {format_duration_us(average)}", "{format_duration_us(summary.total_us)}" }
                span { class: "w-20 shrink-0 text-right font-mono text-[10px] tabular-nums text-blue-700", "{format_duration_us(summary.self_us)}" }
                span { class: "w-14 shrink-0 text-right text-[10px] tabular-nums text-gray-500", "{coverage_percent(summary, window_range_us):.1}%" }
            }
            if expanded() {
                div { class: "border-l border-gray-200", style: "margin-left: {25 + indent}px",
                    for (index, slice) in group.slices.iter().enumerate() {
                        TimelineSliceNode {
                            key: "{slice.pid}-{slice.tid}-{slice.start_us}",
                            slice: slice.clone(),
                            depth: depth + 1,
                            occurrence: Some(index + 1),
                            window_start_us,
                            window_range_us,
                            trace_start_us,
                            expansion,
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn TimelineSliceNode(
    slice: TimelineSlice,
    depth: usize,
    occurrence: Option<usize>,
    window_start_us: f64,
    window_range_us: f64,
    trace_start_us: f64,
    expansion: Signal<ExpansionCommand>,
) -> Element {
    let mut expanded = use_signal(|| false);
    let has_children = !slice.children.is_empty();
    let child_groups = group_slices(&slice.children);
    let duration = format_duration_us(slice.dur_us.max(0.0));
    let self_time = format_duration_us(exclusive_us(&slice));
    let window_pct = slice.dur_us / window_range_us.max(1.0) * 100.0;
    let color = slice_color(&slice.cat);
    let indent = depth * 14;
    let intervals = vec![(slice.start_us, slice.dur_us)];

    sync_expansion(expanded, expansion);

    rsx! {
        div { class: "border-b border-gray-100 last:border-b-0",
            div { class: "flex min-w-0 items-center gap-2 px-3 py-1.5 hover:bg-gray-50", style: "padding-left: {12 + indent}px",
                if has_children {
                    button {
                        class: "flex h-5 w-5 shrink-0 items-center justify-center text-gray-400 hover:text-gray-700",
                        aria_label: if expanded() { "Collapse nested slices" } else { "Expand nested slices" },
                        onclick: move |_| expanded.set(!expanded()),
                        Icon { icon: if expanded() { &icondata::AiCaretDownOutlined } else { &icondata::AiCaretRightOutlined }, class: "h-3 w-3" }
                    }
                } else {
                    span { class: "w-5 shrink-0" }
                }
                div { class: "flex min-w-0 w-64 shrink-0 items-center gap-2",
                    span { class: "h-2.5 w-2.5 shrink-0 rounded-sm {color}" }
                    span { class: "min-w-0 truncate text-xs font-medium text-gray-900", title: "{slice.name}", "{slice.name}" }
                    if let Some(index) = occurrence {
                        span { class: "font-mono text-[9px] text-gray-400", "#{index}" }
                    }
                    if has_children {
                        span { class: "text-[9px] text-gray-400", "{slice.children.len()} children" }
                    }
                }
                TimeStrip {
                    intervals,
                    window_start_us,
                    window_range_us,
                    color,
                    label: "Slice position in its parent window",
                }
                span { class: "w-20 shrink-0 text-right font-mono text-[10px] tabular-nums text-gray-600", "{duration}" }
                span { class: "w-20 shrink-0 text-right font-mono text-[10px] tabular-nums text-blue-700", "{self_time}" }
                span { class: "w-14 shrink-0 text-right text-[10px] tabular-nums text-gray-500", "{window_pct:.1}%" }
            }
            if expanded() {
                SliceDetails { slice: slice.clone(), trace_start_us }
                if has_children {
                    div { class: "border-l border-gray-200", style: "margin-left: {25 + indent}px",
                        for group in child_groups {
                            SliceGroupNode {
                                key: "{group.cat}-{group.name}-{group.slices[0].start_us}",
                                group,
                                depth: depth + 1,
                                window_start_us: slice.start_us,
                                window_range_us: slice.dur_us.max(1.0),
                                trace_start_us,
                                expansion,
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn TimeStrip(
    intervals: Vec<(f64, f64)>,
    window_start_us: f64,
    window_range_us: f64,
    color: &'static str,
    label: &'static str,
) -> Element {
    let segments = intervals
        .iter()
        .filter_map(|(start, duration)| {
            interval_percent(*start, *duration, window_start_us, window_range_us)
        })
        .collect::<Vec<_>>();

    rsx! {
        div { class: "relative h-3 min-w-[160px] flex-1 overflow-hidden rounded-sm bg-gray-100 ring-1 ring-inset ring-gray-200", title: "{label}",
            for (left, width) in segments {
                span {
                    class: "absolute inset-y-[2px] rounded-[1px] opacity-80 {color}",
                    style: "left: {left:.3}%; width: {width:.3}%; min-width: 2px;",
                }
            }
        }
    }
}

#[component]
fn SliceDetails(slice: TimelineSlice, trace_start_us: f64) -> Element {
    let start = format_duration_us((slice.start_us - trace_start_us).max(0.0));
    let end = format_duration_us((slice.end_us() - trace_start_us).max(0.0));
    rsx! {
        div { class: "flex flex-wrap items-center gap-x-3 gap-y-1 border-t border-gray-100 bg-gray-50/60 px-10 py-1.5 text-[9px] text-gray-500",
            span { "start +{start}" }
            span { "end +{end}" }
            span { "pid {slice.pid}" }
            span { "tid {slice.tid}" }
            span { "category {slice.cat}" }
            if let Some(args) = slice.args {
                span { class: "min-w-0 flex-1 truncate font-mono text-gray-600", title: "{args}", "{args}" }
            }
        }
    }
}

fn sync_expansion(mut expanded: Signal<bool>, expansion: Signal<ExpansionCommand>) {
    use_effect(move || {
        let command = expansion();
        if command.revision > 0 {
            expanded.set(command.expanded);
        }
    });
}

fn group_slices(slices: &[TimelineSlice]) -> Vec<SliceGroup> {
    let mut groups: Vec<SliceGroup> = Vec::new();
    for slice in slices {
        if let Some(group) = groups
            .iter_mut()
            .find(|group| group.name == slice.name && group.cat == slice.cat)
        {
            group.slices.push(slice.clone());
        } else {
            groups.push(SliceGroup {
                name: slice.name.clone(),
                cat: slice.cat.clone(),
                slices: vec![slice.clone()],
            });
        }
    }
    groups
}

fn summarize_slices(
    slices: &[TimelineSlice],
    window_start_us: f64,
    window_range_us: f64,
) -> SliceSummary {
    SliceSummary {
        occurrences: slices.len(),
        descendants: slices.iter().map(TimelineSlice::descendant_count).sum(),
        total_us: slices.iter().map(|slice| slice.dur_us.max(0.0)).sum(),
        self_us: slices.iter().map(exclusive_us).sum(),
        covered_us: covered_us(slices, window_start_us, window_range_us),
    }
}

fn intervals_for(slices: &[TimelineSlice]) -> Vec<(f64, f64)> {
    slices
        .iter()
        .map(|slice| (slice.start_us, slice.dur_us))
        .collect()
}

fn coverage_percent(summary: SliceSummary, window_range_us: f64) -> f64 {
    summary.covered_us / window_range_us.max(1.0) * 100.0
}

fn covered_us(slices: &[TimelineSlice], window_start_us: f64, window_range_us: f64) -> f64 {
    let window_end_us = window_start_us + window_range_us.max(1.0);
    let intervals = slices.iter().filter_map(|slice| {
        let start = slice.start_us.max(window_start_us);
        let end = slice.end_us().min(window_end_us);
        (end > start).then_some((start, end))
    });
    union_length(intervals)
}

fn interval_percent(
    start_us: f64,
    duration_us: f64,
    window_start_us: f64,
    window_range_us: f64,
) -> Option<(f64, f64)> {
    let range = window_range_us.max(1.0);
    let window_end = window_start_us + range;
    let start = start_us.max(window_start_us);
    let end = (start_us + duration_us.max(0.0)).min(window_end);
    if end <= start {
        return None;
    }
    let left = (start - window_start_us) / range * 100.0;
    let width = ((end - start) / range * 100.0).max(0.15);
    Some((left.clamp(0.0, 100.0), width.min(100.0 - left)))
}

pub(super) fn exclusive_us(slice: &TimelineSlice) -> f64 {
    let intervals = slice.children.iter().filter_map(|child| {
        let start = child.start_us.max(slice.start_us);
        let end = child.end_us().min(slice.end_us());
        (end > start).then_some((start, end))
    });
    (slice.dur_us - union_length(intervals)).max(0.0)
}

fn union_length(intervals: impl Iterator<Item = (f64, f64)>) -> f64 {
    let mut intervals = intervals.collect::<Vec<_>>();
    intervals.sort_by(|a, b| a.0.total_cmp(&b.0));

    let mut covered = 0.0;
    let mut current: Option<(f64, f64)> = None;
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

fn toolbar_button() -> &'static str {
    "rounded-md border border-gray-300 bg-white px-2 py-1.5 text-xs text-gray-600 hover:bg-gray-50"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named_slice(
        name: &str,
        cat: &str,
        start: f64,
        duration: f64,
        children: Vec<TimelineSlice>,
    ) -> TimelineSlice {
        TimelineSlice {
            name: name.into(),
            cat: cat.into(),
            start_us: start,
            dur_us: duration,
            pid: 1,
            tid: 2,
            args: None,
            children,
        }
    }

    fn slice(start: f64, duration: f64, children: Vec<TimelineSlice>) -> TimelineSlice {
        named_slice("slice", "test", start, duration, children)
    }

    #[test]
    fn exclusive_time_unions_overlapping_children() {
        let parent = slice(
            0.0,
            100.0,
            vec![slice(10.0, 40.0, vec![]), slice(30.0, 40.0, vec![])],
        );
        assert_eq!(exclusive_us(&parent), 40.0);
    }

    #[test]
    fn repeated_siblings_group_without_flattening_children() {
        let backward_child = named_slice("gemm", "kernel", 2.0, 3.0, vec![]);
        let slices = vec![
            named_slice("backward", "phase", 0.0, 10.0, vec![backward_child]),
            named_slice("optimizer", "phase", 10.0, 5.0, vec![]),
            named_slice("backward", "phase", 20.0, 8.0, vec![]),
        ];

        let groups = group_slices(&slices);

        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].name, "backward");
        assert_eq!(groups[0].slices.len(), 2);
        assert_eq!(groups[0].slices[0].children[0].name, "gemm");
        assert_eq!(groups[1].name, "optimizer");
    }

    #[test]
    fn collapsed_summary_keeps_count_total_self_and_union_coverage() {
        let child = slice(10.0, 20.0, vec![]);
        let slices = vec![slice(0.0, 40.0, vec![child]), slice(30.0, 40.0, vec![])];

        let summary = summarize_slices(&slices, 0.0, 100.0);

        assert_eq!(summary.occurrences, 2);
        assert_eq!(summary.descendants, 3);
        assert_eq!(summary.total_us, 80.0);
        assert_eq!(summary.self_us, 60.0);
        assert_eq!(summary.covered_us, 70.0);
        assert_eq!(coverage_percent(summary, 100.0), 70.0);
    }

    #[test]
    fn time_strip_clips_intervals_to_parent_window() {
        assert_eq!(
            interval_percent(90.0, 20.0, 100.0, 100.0),
            Some((0.0, 10.0))
        );
        assert_eq!(
            interval_percent(180.0, 40.0, 100.0, 100.0),
            Some((80.0, 20.0))
        );
        assert_eq!(interval_percent(0.0, 10.0, 100.0, 100.0), None);
    }
}
