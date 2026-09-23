use dioxus::prelude::*;
use dioxus_router::use_navigator;
use probing_proto::prelude::{RlRunSummary, RlSeriesResponse};
use std::collections::BTreeMap;

use crate::api::ApiClient;
use crate::components::rl::metrics_line_chart::{
    ChartPoint, ChartSeries, ChartXAxis, MetricsLineChart,
};
use crate::hooks::{use_page_visible, use_polled_resource};
use crate::state::rl::SAMPLE_STEP_FILTER;
use crate::utils::error::Result;
use crate::utils::metric_format::{format_metric_delta, format_metric_value};

use super::super::components::{LoadingPanel, UnavailablePanel, WorkspacePage};
use super::super::rl_run::{read_selected_run_id, resolve_run, write_selected_run_id};
use super::super::routes::NextRoute;
use super::rl_overview::metric_tooltip;

const POLL_MS: u32 = 10_000;
const PAGE_SIZE: usize = 18;
const SERIES_LIMIT: usize = 8_000;
const MAX_COMPARE_RUNS: usize = 2;
/// Widest trailing-average window the smoothing slider offers.
const MAX_SMOOTH_WINDOW: usize = 20;
const COLORS: [&str; 6] = [
    "#2563eb", "#dc2626", "#16a34a", "#9333ea", "#ea580c", "#0891b2",
];

#[derive(Clone, Debug, PartialEq)]
struct RlMetricsEvidence {
    runs: Vec<RlRunSummary>,
    run_id: String,
    tags: Vec<String>,
    series_by_run: BTreeMap<String, RlSeriesResponse>,
}

#[component]
pub fn RlMetricsPage() -> Element {
    let visible = use_page_visible();
    let mut selected_run_id = use_signal(|| read_selected_run_id().unwrap_or_default());
    let mut compare_run_ids = use_signal(Vec::<String>::new);
    let query = use_signal(String::new);
    let group = use_signal(|| "all".to_string());
    let mut page = use_signal(|| 0_usize);
    let smooth_window = use_signal(|| 1_usize);
    let log_scale = use_signal(|| false);
    let mut x_axis = use_signal(|| ChartXAxis::Step);
    let expanded_groups = use_signal(BTreeMap::<String, bool>::new);
    let evidence = use_polled_resource(POLL_MS, Some(visible), move || {
        let preferred = selected_run_id();
        let compare = compare_run_ids();
        async move { load_metrics(preferred, compare).await }
    });
    let state = evidence.read().clone();

    rsx! {
        WorkspacePage {
            title: "RL Metrics".to_string(),
            subtitle: "Tag tree, multi-run overlay, and step/time axis with sample drill-down.".to_string(),
            actions: rsx! {
                div { class: "flex items-center gap-2 text-xs text-gray-500",
                    span { "Live · {POLL_MS / 1000}s" }
                    select {
                        class: "rounded border border-gray-300 bg-white px-2 py-1 text-xs text-gray-700",
                        value: if matches!(x_axis(), ChartXAxis::Step) { "step" } else { "time" },
                        onchange: move |event| {
                            x_axis.set(if event.value() == "time" {
                                ChartXAxis::Time
                            } else {
                                ChartXAxis::Step
                            });
                        },
                        option { value: "step", "X: step" }
                        option { value: "time", "X: time" }
                    }
                }
            },
            match state {
                None => rsx! { LoadingPanel { label: "Loading RL metric catalog".to_string() } },
                Some(Err(error)) => rsx! { UnavailablePanel {
                    label: "RL metrics unavailable".to_string(),
                    detail: error.display_message(),
                }},
                Some(Ok(evidence)) if evidence.tags.is_empty() => rsx! { UnavailablePanel {
                    label: "No RL metric tags reported".to_string(),
                    detail: "Enable a framework adapter and wait for its first completed trainer step.".to_string(),
                }},
                Some(Ok(evidence)) => rsx! {
                    MetricsWorkbench {
                        evidence,
                        selected_run_id,
                        compare_run_ids,
                        query,
                        group,
                        page,
                        smooth_window,
                        log_scale,
                        x_axis: x_axis(),
                        expanded_groups,
                        on_select_run: move |run_id: String| {
                            write_selected_run_id(&run_id);
                            selected_run_id.set(run_id.clone());
                            compare_run_ids.set(
                                compare_run_ids()
                                    .into_iter()
                                    .filter(|candidate| candidate != &run_id)
                                    .collect(),
                            );
                            page.set(0);
                        },
                    }
                },
            }
        }
    }
}

async fn load_metrics(preferred: String, compare: Vec<String>) -> Result<RlMetricsEvidence> {
    let client = ApiClient::new();
    let runs = client.fetch_rl_runs().await?.runs;
    let Some(run) = resolve_run(&runs, Some(preferred.as_str())).cloned() else {
        return Ok(RlMetricsEvidence {
            runs,
            run_id: String::new(),
            tags: Vec::new(),
            series_by_run: BTreeMap::new(),
        });
    };
    let run_id = run.run_id.clone();
    let tags = client.fetch_rl_tags(&run_id).await?.tags;
    let mut run_ids = vec![run_id.clone()];
    for candidate in compare {
        if candidate != run_id
            && !run_ids.contains(&candidate)
            && run_ids.len() < 1 + MAX_COMPARE_RUNS
            && runs.iter().any(|item| item.run_id == candidate)
        {
            run_ids.push(candidate);
        }
    }
    let mut series_by_run = BTreeMap::new();
    for id in run_ids {
        series_by_run.insert(id.clone(), fetch_series_chunks(&client, &id, &tags).await?);
    }
    Ok(RlMetricsEvidence {
        runs,
        run_id,
        tags,
        series_by_run,
    })
}

async fn fetch_series_chunks(
    client: &ApiClient,
    run_id: &str,
    tags: &[String],
) -> Result<RlSeriesResponse> {
    if tags.is_empty() {
        return Ok(RlSeriesResponse {
            run_id: run_id.to_string(),
            ..Default::default()
        });
    }
    let mut merged = RlSeriesResponse {
        run_id: run_id.to_string(),
        ..Default::default()
    };
    let names = tags.iter().map(String::as_str).collect::<Vec<_>>();
    for chunk in names.chunks(64) {
        let partial = client.fetch_rl_series(run_id, chunk, SERIES_LIMIT).await?;
        merged.series.extend(partial.series);
    }
    Ok(merged)
}

#[component]
fn MetricsWorkbench(
    evidence: RlMetricsEvidence,
    selected_run_id: Signal<String>,
    compare_run_ids: Signal<Vec<String>>,
    query: Signal<String>,
    group: Signal<String>,
    page: Signal<usize>,
    smooth_window: Signal<usize>,
    log_scale: Signal<bool>,
    x_axis: ChartXAxis,
    expanded_groups: Signal<BTreeMap<String, bool>>,
    on_select_run: EventHandler<String>,
) -> Element {
    let navigator = use_navigator();
    let current = selected_run_id();
    let selected_compare = compare_run_ids();
    let groups = tag_tree(&evidence.tags);
    let filtered = filter_tags(&evidence.tags, &query(), &group());
    let page_count = filtered.len().div_ceil(PAGE_SIZE).max(1);
    let page_idx = page().min(page_count - 1);
    let start = page_idx * PAGE_SIZE;
    let page_tags = filtered
        .iter()
        .skip(start)
        .take(PAGE_SIZE)
        .cloned()
        .collect::<Vec<_>>();
    let showing = format!(
        "Showing {}-{} of {} tags",
        if filtered.is_empty() { 0 } else { start + 1 },
        (start + page_tags.len()).min(filtered.len()),
        filtered.len()
    );
    let compare_candidates = evidence
        .runs
        .iter()
        .filter(|run| run.run_id != evidence.run_id)
        .cloned()
        .collect::<Vec<_>>();

    rsx! {
        div { class: "mb-3 grid grid-cols-1 gap-3 lg:grid-cols-4",
            label { class: "block text-xs text-gray-600",
                span { class: "mb-1 block font-medium uppercase tracking-wide", "Primary run" }
                select {
                    class: "w-full rounded-md border border-gray-300 px-3 py-2 text-sm",
                    value: "{current}",
                    onchange: move |event| on_select_run.call(event.value()),
                    for run in evidence.runs.iter() {
                        option {
                            value: "{run.run_id}",
                            selected: run.run_id == current || (current.is_empty() && run.run_id == evidence.run_id),
                            "{run.framework} · {run.run_id}"
                        }
                    }
                }
            }
            label { class: "block text-xs text-gray-600 lg:col-span-2",
                span { class: "mb-1 block font-medium uppercase tracking-wide", "Search" }
                input {
                    class: "w-full rounded-md border border-gray-300 px-3 py-2 text-sm",
                    r#type: "search",
                    placeholder: "Filter metric tags",
                    value: "{query}",
                    oninput: move |event| {
                        query.set(event.value());
                        page.set(0);
                    },
                }
            }
            div { class: "flex flex-wrap items-end gap-4 text-xs text-gray-600",
                div { class: "flex overflow-hidden rounded-md border border-gray-300",
                    button {
                        class: if log_scale() { "px-2 py-1 text-gray-600" } else { "bg-gray-800 px-2 py-1 text-white" },
                        onclick: move |_| log_scale.set(false),
                        "linear"
                    }
                    button {
                        class: if log_scale() { "bg-gray-800 px-2 py-1 text-white" } else { "px-2 py-1 text-gray-600" },
                        onclick: move |_| log_scale.set(true),
                        "log"
                    }
                }
                label { class: "inline-flex items-center gap-2",
                    span { "smoothing" }
                    input {
                        r#type: "range",
                        class: "w-24 align-middle",
                        min: "1",
                        max: "{MAX_SMOOTH_WINDOW}",
                        step: "1",
                        value: "{smooth_window()}",
                        oninput: move |event| {
                            if let Ok(window) = event.value().parse::<usize>() {
                                smooth_window.set(window.clamp(1, MAX_SMOOTH_WINDOW));
                            }
                        },
                    }
                    span { class: "w-10 text-gray-500",
                        if smooth_window() > 1 { "{smooth_window()}" } else { "off" }
                    }
                }
                span { class: "text-gray-500", "{showing}" }
            }
        }

        if !compare_candidates.is_empty() {
            div { class: "mb-3 rounded-md border border-dashed border-gray-300 bg-gray-50 px-3 py-2",
                div { class: "mb-2 text-xs font-medium uppercase tracking-wide text-gray-500",
                    "Compare runs (max {MAX_COMPARE_RUNS})"
                }
                div { class: "flex flex-wrap gap-3 text-xs text-gray-700",
                    for run in compare_candidates.iter() {
                        {
                            let run_id = run.run_id.clone();
                            let checked = selected_compare.contains(&run_id);
                            rsx! {
                                label { class: "inline-flex items-center gap-2",
                                    input {
                                        r#type: "checkbox",
                                        checked: checked,
                                        onchange: move |_| {
                                            let mut next = compare_run_ids();
                                            if next.contains(&run_id) {
                                                next.retain(|item| item != &run_id);
                                            } else if next.len() < MAX_COMPARE_RUNS {
                                                next.push(run_id.clone());
                                            }
                                            compare_run_ids.set(next);
                                        },
                                    }
                                    "{run.run_id}"
                                }
                            }
                        }
                    }
                }
            }
        }

        div { class: "grid grid-cols-1 gap-4 xl:grid-cols-[240px_minmax(0,1fr)]",
            div { class: "rounded-lg border border-gray-200 bg-white p-3",
                div { class: "mb-2 flex items-center justify-between gap-2",
                    span { class: "text-xs font-medium uppercase tracking-wide text-gray-500", "Tag tree" }
                    button {
                        class: "text-xs text-blue-600 hover:underline",
                        onclick: move |_| {
                            group.set("all".into());
                            page.set(0);
                        },
                        "All"
                    }
                }
                div { class: "max-h-[70vh] space-y-1 overflow-y-auto text-xs",
                    for (group_name, tags) in groups.iter() {
                        {
                            let group_name = group_name.clone();
                            let expanded = expanded_groups()
                                .get(&group_name)
                                .copied()
                                .unwrap_or(true);
                            let active = group() == group_name;
                            let count = tags.len();
                            rsx! {
                                div { class: "rounded border border-gray-100",
                                    div { class: "flex items-center gap-1 px-2 py-1.5 hover:bg-gray-50",
                                        button {
                                            class: "w-4 shrink-0 text-gray-500",
                                            onclick: {
                                                let group_name = group_name.clone();
                                                move |_| {
                                                    let mut map = expanded_groups();
                                                    let next = !map.get(&group_name).copied().unwrap_or(true);
                                                    map.insert(group_name.clone(), next);
                                                    expanded_groups.set(map);
                                                }
                                            },
                                            if expanded { "▾" } else { "▸" }
                                        }
                                        button {
                                            class: if active {
                                                "min-w-0 flex-1 truncate text-left font-medium text-blue-700"
                                            } else {
                                                "min-w-0 flex-1 truncate text-left font-medium text-gray-800"
                                            },
                                            onclick: {
                                                let group_name = group_name.clone();
                                                move |_| {
                                                    group.set(group_name.clone());
                                                    page.set(0);
                                                }
                                            },
                                            "{group_name} ({count})"
                                        }
                                    }
                                    if expanded {
                                        div { class: "space-y-0.5 border-t border-gray-100 px-2 py-1",
                                            for tag in tags.iter() {
                                                {
                                                    let tag = tag.clone();
                                                    let selected = query().eq(&tag);
                                                    rsx! {
                                                        button {
                                                            class: if selected {
                                                                "block w-full truncate rounded px-2 py-1 text-left text-blue-700 bg-blue-50"
                                                            } else {
                                                                "block w-full truncate rounded px-2 py-1 text-left text-gray-600 hover:bg-gray-50"
                                                            },
                                                            onclick: move |_| {
                                                                query.set(tag.clone());
                                                                group.set("all".into());
                                                                page.set(0);
                                                            },
                                                            "{tag}"
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
                }
            }

            div {
                if page_count > 1 {
                    div { class: "mb-3 flex items-center justify-end gap-2",
                        button {
                            class: "rounded border border-gray-300 px-2 py-1 text-xs disabled:opacity-40",
                            disabled: page_idx == 0,
                            onclick: move |_| page.set(page_idx.saturating_sub(1)),
                            "Prev"
                        }
                        span { class: "text-xs text-gray-500", "Page {page_idx + 1} / {page_count}" }
                        button {
                            class: "rounded border border-gray-300 px-2 py-1 text-xs disabled:opacity-40",
                            disabled: page_idx + 1 >= page_count,
                            onclick: move |_| page.set(page_idx + 1),
                            "Next"
                        }
                    }
                }
                if page_tags.is_empty() {
                    UnavailablePanel {
                        label: "No metrics match the current filters".to_string(),
                        detail: "Clear the search box or pick another tag group.".to_string(),
                    }
                } else {
                    div { class: "grid grid-cols-1 gap-3 lg:grid-cols-2 2xl:grid-cols-3",
                        for name in page_tags.iter() {
                            MetricsLineChart {
                                title: name.clone(),
                                series: compare_series(&evidence, name, smooth_window(), x_axis),
                                x_axis,
                                log_scale: log_scale(),
                                height: 200.0,
                                subtitle: series_stats(
                                    evidence.series_by_run.get(&evidence.run_id),
                                    name,
                                ),
                                tooltip: metric_tooltip(name),
                                on_point_click: move |step| {
                                    if step >= 0 {
                                        *SAMPLE_STEP_FILTER.write() = Some(step);
                                        navigator.push(NextRoute::RlSamples {});
                                    }
                                },
                            }
                        }
                    }
                }
            }
        }
    }
}

fn compare_series(
    evidence: &RlMetricsEvidence,
    name: &str,
    smooth_window: usize,
    x_axis: ChartXAxis,
) -> Vec<ChartSeries> {
    let mut ordered = vec![evidence.run_id.clone()];
    for run_id in evidence.series_by_run.keys() {
        if run_id != &evidence.run_id {
            ordered.push(run_id.clone());
        }
    }
    ordered
        .into_iter()
        .enumerate()
        .filter_map(|(index, run_id)| {
            let response = evidence.series_by_run.get(&run_id)?;
            let points = response.series.get(name)?;
            let values = points
                .iter()
                .map(|point| ChartPoint {
                    x: match x_axis {
                        ChartXAxis::Step => point.step as f64,
                        ChartXAxis::Time => {
                            if point.timestamp_ns > 0 {
                                point.timestamp_ns as f64
                            } else {
                                point.wall_time_s
                            }
                        }
                    },
                    y: point.value,
                    step: point.step,
                    ..Default::default()
                })
                .collect::<Vec<_>>();
            // Read out the raw tail, not the smoothed one, so the legend keeps
            // reporting what the trainer actually reported.
            let readout = points
                .last()
                .map(|point| format_metric_value(name, point.value))
                .unwrap_or_default();
            let delta = if points.len() >= 2 {
                format_metric_delta(
                    name,
                    points[points.len() - 1].value - points[points.len() - 2].value,
                )
            } else {
                String::new()
            };
            let values = if smooth_window > 1 {
                smooth_points(&values, smooth_window)
            } else {
                values
            };
            Some(ChartSeries {
                label: run_id,
                points: values,
                color: COLORS[index % COLORS.len()],
                readout,
                delta,
            })
        })
        .collect()
}

fn tag_tree(tags: &[String]) -> Vec<(String, Vec<String>)> {
    let mut tree: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for tag in tags {
        tree.entry(tag_group(tag).to_string())
            .or_default()
            .push(tag.clone());
    }
    tree.into_iter().collect()
}

fn tag_group(tag: &str) -> &str {
    tag.split_once('.').map(|(prefix, _)| prefix).unwrap_or(tag)
}

fn filter_tags(tags: &[String], query: &str, group: &str) -> Vec<String> {
    let needle = query.trim().to_ascii_lowercase();
    tags.iter()
        .filter(|tag| group == "all" || tag_group(tag) == group)
        .filter(|tag| needle.is_empty() || tag.to_ascii_lowercase().contains(&needle))
        .cloned()
        .collect()
}

fn smooth_points(points: &[ChartPoint], window: usize) -> Vec<ChartPoint> {
    if window <= 1 || points.len() < window {
        return points.to_vec();
    }
    points
        .iter()
        .enumerate()
        .map(|(index, point)| {
            let start = index.saturating_sub(window - 1);
            let slice = &points[start..=index];
            let mean = slice.iter().map(|item| item.y).sum::<f64>() / slice.len() as f64;
            ChartPoint {
                x: point.x,
                y: mean,
                step: point.step,
                ..Default::default()
            }
        })
        .collect()
}

fn series_stats(response: Option<&RlSeriesResponse>, name: &str) -> String {
    let Some(response) = response else {
        return String::new();
    };
    let Some(points) = response.series.get(name) else {
        return String::new();
    };
    if points.is_empty() {
        return String::new();
    }
    let values = points.iter().map(|point| point.value).collect::<Vec<_>>();
    let min = values.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    format!(
        "min {} · mean {} · max {}",
        format_metric(min),
        format_metric(mean),
        format_metric(max)
    )
}

fn format_metric(value: f64) -> String {
    if value.abs() >= 1_000.0 {
        format!("{value:.0}")
    } else if value.abs() >= 10.0 {
        format!("{value:.2}")
    } else {
        format!("{value:.4}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use probing_proto::prelude::RlMetricPoint;

    #[test]
    fn filter_tags_respects_group_and_search() {
        let tags = vec![
            "reward.mean".into(),
            "policy.entropy".into(),
            "policy.pg_loss".into(),
        ];
        assert_eq!(
            filter_tags(&tags, "pg", "policy"),
            vec!["policy.pg_loss".to_string()]
        );
    }

    #[test]
    fn smooth_points_averages_recent_window() {
        let points = vec![
            ChartPoint {
                x: 1.0,
                y: 1.0,
                step: 1,
                ..Default::default()
            },
            ChartPoint {
                x: 2.0,
                y: 3.0,
                step: 2,
                ..Default::default()
            },
            ChartPoint {
                x: 3.0,
                y: 5.0,
                step: 3,
                ..Default::default()
            },
        ];
        let smoothed = smooth_points(&points, 2);
        assert_eq!(smoothed[0].y, 1.0);
        assert_eq!(smoothed[1].y, 2.0);
        assert_eq!(smoothed[2].y, 4.0);
    }

    #[test]
    fn compare_series_overlays_runs() {
        let mut series_by_run = BTreeMap::new();
        series_by_run.insert(
            "a".into(),
            RlSeriesResponse {
                run_id: "a".into(),
                series: BTreeMap::from([(
                    "reward.mean".into(),
                    vec![RlMetricPoint {
                        step: 1,
                        wall_time_s: 1.0,
                        timestamp_ns: 1,
                        value: 0.2,
                        ..Default::default()
                    }],
                )]),
                ..Default::default()
            },
        );
        series_by_run.insert(
            "b".into(),
            RlSeriesResponse {
                run_id: "b".into(),
                series: BTreeMap::from([(
                    "reward.mean".into(),
                    vec![RlMetricPoint {
                        step: 1,
                        wall_time_s: 1.0,
                        timestamp_ns: 1,
                        value: 0.4,
                        ..Default::default()
                    }],
                )]),
                ..Default::default()
            },
        );
        let evidence = RlMetricsEvidence {
            runs: Vec::new(),
            run_id: "a".into(),
            tags: vec!["reward.mean".into()],
            series_by_run,
        };
        let series = compare_series(&evidence, "reward.mean", 1, ChartXAxis::Step);
        assert_eq!(series.len(), 2);
        assert_eq!(series[0].label, "a");
        assert_eq!(series[1].points[0].y, 0.4);
        // Legend readouts come from the raw tail of each run's series.
        assert_eq!(series[1].readout, "0.4000");
    }

    #[test]
    fn tag_tree_groups_by_prefix() {
        let tree = tag_tree(&[
            "reward.mean".into(),
            "policy.entropy".into(),
            "policy.pg_loss".into(),
        ]);
        assert_eq!(tree.len(), 2);
        assert_eq!(tree[0].0, "policy");
        assert_eq!(tree[0].1.len(), 2);
    }
}
