use std::collections::BTreeMap;

use dioxus::prelude::*;
use probing_proto::prelude::{DataFrame, Ele};

use crate::api::ApiClient;
use crate::hooks::{use_page_visible, use_poll_tick_gated};
use crate::utils::base_path::with_base;

use super::super::components::{
    EvidenceMetric, EvidenceSection, EvidenceSurface, InlineNotice, LoadingPanel, NoticeTone,
    UnavailablePanel, WorkspacePage,
};
use super::super::settings::MEMORY_REFRESH;

const POLL_MS: u32 = 5_000;

const LATEST_SQL: &str = r#"
SELECT ts, pid, actor_class, actor_name, state, rss_bytes, uss_bytes, pss_bytes, threads
FROM rl.actor_memory
WHERE ts >= (SELECT max(ts) FROM rl.actor_memory) - 60000000
ORDER BY ts DESC
LIMIT 2000
"#;

const HISTORY_SQL: &str = r#"
SELECT ts, pid, actor_class, actor_name, uss_bytes
FROM rl.actor_memory
WHERE ts >= (SELECT max(ts) FROM rl.actor_memory) - 600000000
ORDER BY ts ASC
LIMIT 5000
"#;

#[derive(Clone, Debug, PartialEq)]
struct HostSample {
    ts: i64,
    pid: i32,
    actor_class: String,
    actor_name: String,
    state: String,
    rss: u64,
    uss: u64,
    pss: u64,
    threads: i32,
}

#[component]
pub fn HostMemoryPage() -> Element {
    let visible = use_page_visible();
    let poll = use_poll_tick_gated(POLL_MS, Some(visible));
    let refresh_key = use_memo(move || poll().wrapping_add(*MEMORY_REFRESH.read()));

    let latest = use_resource(move || {
        let _ = refresh_key();
        async move { ApiClient::new().execute_query(LATEST_SQL).await }
    });
    let history = use_resource(move || {
        let _ = refresh_key();
        async move { ApiClient::new().execute_query(HISTORY_SQL).await }
    });

    let latest_state = latest.read().clone();
    let history_state = history.read().clone();
    let actors = latest_state
        .as_ref()
        .and_then(|state| state.as_ref().ok())
        .map(|df| latest_by_pid(parse_samples(df)))
        .unwrap_or_default();
    let points = history_state
        .as_ref()
        .and_then(|state| state.as_ref().ok())
        .map(|df| {
            let mut samples = parse_samples(df);
            samples.sort_by_key(|sample| sample.ts);
            samples
        })
        .unwrap_or_default();
    let error_message = latest_state
        .as_ref()
        .and_then(|state| state.as_ref().err())
        .map(|error| error.display_message());
    let missing_table = error_message
        .as_deref()
        .is_some_and(|message| message.to_ascii_lowercase().contains("not found"));
    let loaded = latest_state.as_ref().is_some_and(|state| state.is_ok());

    rsx! {
        WorkspacePage {
            title: "Host Memory".to_string(),
            subtitle: "Per-actor host RSS/USS/PSS from the driver-side rl.actor_memory sampler. RSS includes shared pages; use USS/PSS for private memory.".to_string(),
            actions: Some(rsx! {
                a {
                    href: with_base("/memory"),
                    class: "rounded-lg border border-gray-300 bg-white px-3 py-2 text-xs font-medium text-gray-700 hover:border-gray-400 hover:bg-gray-50",
                    "Device memory"
                }
                button {
                    r#type: "button",
                    class: "rounded-lg border border-gray-300 bg-white px-3 py-2 text-xs font-medium text-gray-700 hover:border-gray-400 hover:bg-gray-50",
                    onclick: move |_| *MEMORY_REFRESH.write() += 1,
                    "Refresh"
                }
            }),
            if let Some(error) = error_message.clone() {
                InlineNotice {
                    title: "Host memory source unavailable".to_string(),
                    detail: error.clone(),
                    tone: NoticeTone::Warning,
                }
                if missing_table {
                    InlineNotice {
                        title: "Install the driver-side actor sampler".to_string(),
                        detail: "In the Ray driver process run: probing -t <driver_pid> eval \"exec(open('examples/rl/actor_mem_sampler.py').read())\". It samples local Ray actors every 5 seconds.".to_string(),
                        tone: NoticeTone::Info,
                    }
                }
            } else if !loaded {
                LoadingPanel { label: "Loading host memory".to_string() }
            } else if actors.is_empty() {
                InlineNotice {
                    title: "Sampler installed, no samples yet".to_string(),
                    detail: "rl.actor_memory is empty. The driver-side actor_mem_sampler writes one row per local actor every 5 seconds; refresh in a few seconds.".to_string(),
                    tone: NoticeTone::Info,
                }
            } else {
                EvidenceSurface {
                    EvidenceSection {
                        title: "Actors".to_string(),
                        subtitle: Some("Latest sample per Ray actor PID; totals are the sum of those latest rows.".to_string()),
                        HostSummary { actors: actors.clone() }
                    }
                    EvidenceSection {
                        title: "USS timeline".to_string(),
                        subtitle: Some("Last 10 minutes of unique-set size per actor. Δ view selects the 12 actors with the largest change; Absolute selects by latest USS.".to_string()),
                        divided: true,
                        HostTimeline { points, actors: actors.clone() }
                    }
                    EvidenceSection {
                        title: "Latest samples".to_string(),
                        subtitle: Some("Newest row for each actor in the last 60 seconds, sorted by USS.".to_string()),
                        body_class: "p-0".to_string(),
                        divided: true,
                        HostTable { actors }
                    }
                }
            }
        }
    }
}

#[component]
fn HostSummary(actors: Vec<HostSample>) -> Element {
    let total_rss = actors.iter().map(|actor| actor.rss).sum::<u64>();
    let total_uss = actors.iter().map(|actor| actor.uss).sum::<u64>();
    let total_pss = actors.iter().map(|actor| actor.pss).sum::<u64>();
    let top = actors.first();
    rsx! {
        div { class: "grid grid-cols-2 gap-4 md:grid-cols-5",
            EvidenceMetric {
                label: "Actors".to_string(),
                value: actors.len().to_string(),
                detail: Some("latest row per PID".to_string()),
            }
            EvidenceMetric {
                label: "Total RSS".to_string(),
                value: format_bytes(total_rss),
                detail: Some("includes shared pages".to_string()),
            }
            EvidenceMetric {
                label: "Total USS".to_string(),
                value: format_bytes(total_uss),
                detail: Some("private memory".to_string()),
            }
            EvidenceMetric {
                label: "Total PSS".to_string(),
                value: format_bytes(total_pss),
                detail: Some("shared pages apportioned".to_string()),
            }
            EvidenceMetric {
                label: "Largest USS".to_string(),
                value: top.map(|actor| format_bytes(actor.uss)).unwrap_or_else(|| "—".to_string()),
                detail: top.map(actor_label),
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HostChartMode {
    Delta,
    Absolute,
}

#[component]
fn HostTimeline(points: Vec<HostSample>, actors: Vec<HostSample>) -> Element {
    let mut mode = use_signal(|| HostChartMode::Delta);
    let current_mode = mode();

    if points.is_empty() {
        return rsx! { UnavailablePanel {
            label: "No timeline samples".to_string(),
            detail: "rl.actor_memory has no rows in the last 10 minutes.".to_string(),
        }};
    }

    let mut series = BTreeMap::<i32, Vec<(i64, u64)>>::new();
    for point in &points {
        series
            .entry(point.pid)
            .or_default()
            .push((point.ts, point.uss));
    }

    let activity = |pid: i32| -> u64 {
        series
            .get(&pid)
            .map(|values| {
                let min = values.iter().map(|(_, uss)| *uss).min().unwrap_or(0);
                let max = values.iter().map(|(_, uss)| *uss).max().unwrap_or(0);
                max.saturating_sub(min)
            })
            .unwrap_or(0)
    };

    let mut selected = actors
        .iter()
        .filter(|actor| series.contains_key(&actor.pid))
        .cloned()
        .collect::<Vec<_>>();
    if current_mode == HostChartMode::Delta {
        selected.sort_by_key(|actor| std::cmp::Reverse(activity(actor.pid)));
    }
    selected.truncate(12);
    selected.sort_by_key(|actor| actor.pid);

    if selected.is_empty() {
        return rsx! { UnavailablePanel {
            label: "No actor series".to_string(),
            detail: "rl.actor_memory has samples but no actor is present in the current window.".to_string(),
        }};
    }

    let mut scale_min = 0i64;
    let mut scale_max = 0i64;
    if current_mode == HostChartMode::Absolute {
        scale_min = i64::MAX;
        scale_max = i64::MIN;
    }
    for actor in &selected {
        let Some(values) = series.get(&actor.pid) else {
            continue;
        };
        let base = values
            .first()
            .map(|(_, uss)| *uss as i64)
            .unwrap_or_default();
        for (_, uss) in values {
            let value = if current_mode == HostChartMode::Delta {
                *uss as i64 - base
            } else {
                *uss as i64
            };
            scale_min = scale_min.min(value);
            scale_max = scale_max.max(value);
        }
    }
    if scale_min == i64::MAX {
        scale_min = 0;
        scale_max = 1;
    }
    let span = (scale_max - scale_min).max(1);
    let padding = ((span as f64) * 0.15).ceil() as i64;
    let scale_min = scale_min - padding;
    let scale_max = scale_max + padding;
    let range = (scale_max - scale_min).max(1) as f64;

    let start = points.first().map(|point| point.ts).unwrap_or_default();
    let end = points.last().map(|point| point.ts).unwrap_or(start);
    let time_span = (end - start).max(1) as f64;
    let width = 920.0_f64;
    let height = 200.0_f64;
    let zero_y = (1.0 - (0.0 - scale_min as f64) / range) * height;

    let lines = selected
        .iter()
        .filter_map(|actor| {
            let values = series.get(&actor.pid)?;
            let base = values
                .first()
                .map(|(_, uss)| *uss as i64)
                .unwrap_or_default();
            let polyline = values
                .iter()
                .map(|(ts, uss)| {
                    let value = if current_mode == HostChartMode::Delta {
                        *uss as i64 - base
                    } else {
                        *uss as i64
                    };
                    let x = (ts - start) as f64 / time_span * width;
                    let y = (1.0 - (value as f64 - scale_min as f64) / range) * height;
                    format!("{x:.1},{y:.1}")
                })
                .collect::<Vec<_>>()
                .join(" ");
            Some((actor.clone(), polyline, color_for_pid(actor.pid)))
        })
        .collect::<Vec<_>>();
    let hidden = actors.len().saturating_sub(selected.len());
    let mode_title = if current_mode == HostChartMode::Delta {
        "Δ USS from window start"
    } else {
        "USS (absolute)"
    };
    let range_text = if current_mode == HostChartMode::Delta {
        format!(
            "range {} … {}",
            format_delta(scale_min),
            format_delta(scale_max)
        )
    } else {
        format!(
            "range {} … {}",
            format_bytes(scale_min.max(0) as u64),
            format_bytes(scale_max.max(0) as u64)
        )
    };

    rsx! {
        div {
            div { class: "mb-3 flex flex-wrap items-start justify-between gap-3",
                div { class: "min-w-0",
                    div { class: "text-xs font-medium text-gray-700", "{mode_title}" }
                    div { class: "mt-1 text-xs text-gray-500",
                        "{range_text} · window {format_seconds(end - start)} · top {selected.len()} of {actors.len()} actors"
                    }
                }
                div { class: "inline-flex rounded-lg border border-gray-300 bg-gray-50 p-0.5",
                    button {
                        r#type: "button",
                        class: if current_mode == HostChartMode::Delta {
                            "rounded-md bg-blue-600 px-2.5 py-1 text-xs font-medium text-white shadow-sm"
                        } else {
                            "rounded-md px-2.5 py-1 text-xs font-medium text-gray-600 hover:bg-white"
                        },
                        aria_pressed: if current_mode == HostChartMode::Delta { "true" } else { "false" },
                        title: "Change in USS from the first sample in the window",
                        onclick: move |_| mode.set(HostChartMode::Delta),
                        "Δ USS"
                    }
                    button {
                        r#type: "button",
                        class: if current_mode == HostChartMode::Absolute {
                            "rounded-md bg-blue-600 px-2.5 py-1 text-xs font-medium text-white shadow-sm"
                        } else {
                            "rounded-md px-2.5 py-1 text-xs font-medium text-gray-600 hover:bg-white"
                        },
                        aria_pressed: if current_mode == HostChartMode::Absolute { "true" } else { "false" },
                        title: "Absolute USS",
                        onclick: move |_| mode.set(HostChartMode::Absolute),
                        "Absolute"
                    }
                }
            }
            div {
                class: "relative h-52",
                role: "img",
                aria_label: if current_mode == HostChartMode::Delta {
                    "Per-actor USS change over the last 10 minutes"
                } else {
                    "Per-actor USS over the last 10 minutes"
                },
                svg {
                    class: "h-full w-full",
                    view_box: "0 0 {width} {height}",
                    preserve_aspect_ratio: "none",
                    for tick in 0..=4 {
                        {
                            let y = tick as f64 / 4.0 * height;
                            rsx! {
                                line {
                                    x1: "0",
                                    y1: "{y}",
                                    x2: "{width}",
                                    y2: "{y}",
                                    stroke: "#e5e7eb",
                                    stroke_width: "1",
                                    vector_effect: "non-scaling-stroke",
                                }
                            }
                        }
                    }
                    if current_mode == HostChartMode::Delta {
                        line {
                            x1: "0",
                            y1: "{zero_y}",
                            x2: "{width}",
                            y2: "{zero_y}",
                            stroke: "#94a3b8",
                            stroke_width: "1",
                            stroke_dasharray: "4 3",
                            vector_effect: "non-scaling-stroke",
                        }
                    }
                    for (_, polyline, color) in lines.iter() {
                        polyline {
                            points: "{polyline}",
                            fill: "none",
                            stroke: "{color}",
                            stroke_width: "2",
                            stroke_linejoin: "round",
                            stroke_linecap: "round",
                            vector_effect: "non-scaling-stroke",
                        }
                    }
                }
            }
            div { class: "mt-3 flex flex-wrap gap-x-4 gap-y-1 text-xs text-gray-700",
                for (actor, _, color) in lines.iter() {
                    {
                        let delta = series
                            .get(&actor.pid)
                            .map(|values| {
                                values.last().map(|(_, uss)| *uss).unwrap_or_default() as i64
                                    - values.first().map(|(_, uss)| *uss).unwrap_or_default() as i64
                            })
                            .unwrap_or_default();
                        rsx! {
                            span { class: "inline-flex min-w-0 items-center gap-1.5",
                                span { class: "h-2 w-2 shrink-0 rounded-full", style: "background: {color};" }
                                span { class: "truncate", title: "{actor_label(actor)}", "{actor_label(actor)}" }
                                if current_mode == HostChartMode::Delta {
                                    span { class: "tabular-nums text-gray-500", "{format_delta(delta)} · {format_bytes(actor.uss)}" }
                                } else {
                                    span { class: "tabular-nums text-gray-500", "{format_bytes(actor.uss)}" }
                                }
                            }
                        }
                    }
                }
            }
            if hidden > 0 {
                div { class: "mt-2 text-xs text-gray-500", "+{hidden} more actors not drawn; see the latest samples table." }
            }
        }
    }
}

#[component]
fn HostTable(actors: Vec<HostSample>) -> Element {
    let latest_ts = actors
        .iter()
        .map(|actor| actor.ts)
        .max()
        .unwrap_or_default();
    rsx! {
        div { class: "overflow-x-auto",
            table { class: "min-w-full text-left text-xs",
                thead { class: "bg-gray-50 text-gray-500",
                    tr {
                        th { class: "px-4 py-2 font-medium", "Actor" }
                        th { class: "px-4 py-2 font-medium", "PID" }
                        th { class: "px-4 py-2 font-medium", "State" }
                        th { class: "px-4 py-2 text-right font-medium", "RSS" }
                        th { class: "px-4 py-2 text-right font-medium", "USS" }
                        th { class: "px-4 py-2 text-right font-medium", "PSS" }
                        th { class: "px-4 py-2 text-right font-medium", "Threads" }
                        th { class: "px-4 py-2 text-right font-medium", "Age" }
                    }
                }
                tbody { class: "divide-y divide-gray-100",
                    for actor in actors {
                        tr { class: "hover:bg-gray-50",
                            td { class: "px-4 py-2",
                                span { class: "mr-2 inline-block h-2 w-2 rounded-full", style: "background: {color_for_pid(actor.pid)};" }
                                span { class: "font-medium text-gray-900", "{actor.actor_class}" }
                                if !actor.actor_name.is_empty() {
                                    span { class: "ml-1 text-gray-500", "· {actor.actor_name}" }
                                }
                            }
                            td { class: "px-4 py-2 font-mono tabular-nums text-gray-600", "{actor.pid}" }
                            td { class: "px-4 py-2",
                                span {
                                    class: if actor.state.eq_ignore_ascii_case("ALIVE") {
                                        "rounded bg-emerald-50 px-1.5 py-0.5 font-medium text-emerald-700"
                                    } else {
                                        "rounded bg-gray-100 px-1.5 py-0.5 font-medium text-gray-600"
                                    },
                                    "{actor.state}"
                                }
                            }
                            td { class: "px-4 py-2 text-right tabular-nums text-gray-600", "{format_bytes(actor.rss)}" }
                            td { class: "px-4 py-2 text-right tabular-nums font-medium text-gray-900", "{format_bytes(actor.uss)}" }
                            td { class: "px-4 py-2 text-right tabular-nums text-gray-600", "{format_bytes(actor.pss)}" }
                            td { class: "px-4 py-2 text-right tabular-nums text-gray-600", "{actor.threads}" }
                            td { class: "px-4 py-2 text-right tabular-nums text-gray-500", "{format_age(actor.ts, latest_ts)}" }
                        }
                    }
                }
            }
        }
    }
}

fn parse_samples(dataframe: &DataFrame) -> Vec<HostSample> {
    let ts_col = column(dataframe, "ts");
    let pid_col = column(dataframe, "pid");
    let class_col = column(dataframe, "actor_class");
    let name_col = column(dataframe, "actor_name");
    let state_col = column(dataframe, "state");
    let rss_col = column(dataframe, "rss_bytes");
    let uss_col = column(dataframe, "uss_bytes");
    let pss_col = column(dataframe, "pss_bytes");
    let threads_col = column(dataframe, "threads");

    dataframe
        .iter()
        .filter_map(|row| {
            let pid = i32::try_from(ele_i64(row.get(pid_col.unwrap_or(usize::MAX)))?).ok()?;
            Some(HostSample {
                ts: ele_i64(row.get(ts_col.unwrap_or(usize::MAX)))?,
                pid,
                actor_class: ele_text(row.get(class_col.unwrap_or(usize::MAX))),
                actor_name: ele_text(row.get(name_col.unwrap_or(usize::MAX))),
                state: ele_text(row.get(state_col.unwrap_or(usize::MAX))),
                rss: ele_u64(row.get(rss_col.unwrap_or(usize::MAX))).unwrap_or_default(),
                uss: ele_u64(row.get(uss_col.unwrap_or(usize::MAX))).unwrap_or_default(),
                pss: ele_u64(row.get(pss_col.unwrap_or(usize::MAX))).unwrap_or_default(),
                threads: i32::try_from(
                    ele_i64(row.get(threads_col.unwrap_or(usize::MAX))).unwrap_or_default(),
                )
                .unwrap_or_default(),
            })
        })
        .collect()
}

fn latest_by_pid(samples: Vec<HostSample>) -> Vec<HostSample> {
    let mut latest = BTreeMap::<i32, HostSample>::new();
    for sample in samples {
        match latest.get(&sample.pid) {
            Some(existing) if existing.ts >= sample.ts => {}
            _ => {
                latest.insert(sample.pid, sample);
            }
        }
    }
    let mut rows = latest.into_values().collect::<Vec<_>>();
    rows.sort_by(|left, right| right.uss.cmp(&left.uss).then(left.pid.cmp(&right.pid)));
    rows
}

fn actor_label(actor: &HostSample) -> String {
    if actor.actor_name.is_empty() {
        format!("{} · {}", actor.actor_class, actor.pid)
    } else {
        format!(
            "{} · {} · {}",
            actor.actor_class, actor.actor_name, actor.pid
        )
    }
}

fn color_for_pid(pid: i32) -> &'static str {
    const COLORS: [&str; 10] = [
        "#7c3aed", "#2563eb", "#059669", "#d97706", "#dc2626", "#0891b2", "#9333ea", "#65a30d",
        "#e11d48", "#475569",
    ];
    COLORS[(pid.unsigned_abs() as usize) % COLORS.len()]
}

fn column(dataframe: &DataFrame, name: &str) -> Option<usize> {
    dataframe.names.iter().position(|candidate| {
        candidate.eq_ignore_ascii_case(name)
            || candidate
                .to_ascii_lowercase()
                .ends_with(&format!("_{name}"))
    })
}

fn ele_i64(value: Option<&Ele>) -> Option<i64> {
    match value? {
        Ele::I64(value) => Some(*value),
        Ele::I32(value) => Some(i64::from(*value)),
        Ele::F64(value) => Some(*value as i64),
        Ele::F32(value) => Some(*value as i64),
        Ele::DataTime(value) => Some(*value as i64),
        Ele::Text(value) => value.parse().ok(),
        _ => None,
    }
}

fn ele_u64(value: Option<&Ele>) -> Option<u64> {
    ele_i64(value).and_then(|value| u64::try_from(value).ok())
}

fn ele_text(value: Option<&Ele>) -> String {
    match value {
        Some(Ele::Text(value)) | Some(Ele::Url(value)) => value.clone(),
        Some(value) => value.to_string(),
        None => String::new(),
    }
}

fn format_delta(bytes: i64) -> String {
    if bytes == 0 {
        "0 B".to_string()
    } else if bytes > 0 {
        format!("+{}", format_bytes(bytes as u64))
    } else {
        format!("-{}", format_bytes(bytes.unsigned_abs()))
    }
}

fn format_bytes(bytes: u64) -> String {
    match bytes {
        value if value >= 1 << 30 => format!("{:.1} GiB", value as f64 / (1u64 << 30) as f64),
        value if value >= 1 << 20 => format!("{:.1} MiB", value as f64 / (1u64 << 20) as f64),
        value if value >= 1 << 10 => format!("{:.1} KiB", value as f64 / (1u64 << 10) as f64),
        value => format!("{value} B"),
    }
}

fn format_age(timestamp_us: i64, latest_us: i64) -> String {
    let age_seconds = latest_us.saturating_sub(timestamp_us).max(0) / 1_000_000;
    if age_seconds < 2 {
        "now".to_string()
    } else if age_seconds < 60 {
        format!("{age_seconds}s")
    } else {
        format!("{:.1}m", age_seconds as f64 / 60.0)
    }
}

fn format_seconds(duration_us: i64) -> String {
    let seconds = duration_us.max(0) / 1_000_000;
    if seconds >= 60 {
        format!("{:.1}m", seconds as f64 / 60.0)
    } else {
        format!("{seconds}s")
    }
}
