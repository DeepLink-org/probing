//! Dashboard: live CPU/GPU metrics, process info, threads, env vars.

use std::collections::HashMap;

use dioxus::prelude::*;
use dioxus_router::use_navigator;
use probing_proto::prelude::Process;

use crate::app::Route;

use crate::api::{
    format_bytes, format_cpu_ms, format_opt_pct, format_pct, format_rss, gpu_device_label,
    ApiClient, CpuHistorySample, CpuSnapshot, CpuThreadRow, GpuDeviceRow, GpuHistorySample,
    GpuSnapshot,
};
use crate::components::card::Card;
use crate::components::common::{EmptyState, ErrorState, LoadingState};
use crate::components::cpu_threads_table::CpuThreadsTable;
use crate::components::data::KeyValueList;
use crate::components::page::{PageContainer, PageTitle};
use crate::components::poll_status::PollStatusBar;
use crate::components::stat_card::StatCard;
use crate::hooks::{use_api, use_api_with_options, use_page_visible, use_poll_tick_gated, ApiFetchOptions};
use crate::state::investigation::sync_overview_process_context;

const CPU_POLL_MS: u32 = 2000;
const ENV_VARS_PREVIEW: usize = 40;
const THREADS_PREVIEW: usize = 80;

fn refresh_options() -> ApiFetchOptions {
    ApiFetchOptions {
        keep_previous_while_refreshing: true,
    }
}

#[component]
pub fn Dashboard() -> Element {
    let visible = use_page_visible();
    let poll = use_poll_tick_gated(CPU_POLL_MS, Some(visible));
    let refresh = refresh_options();
    let poll_tick = poll();

    let overview = use_api(|| {
        let client = ApiClient::new();
        async move { client.get_overview().await }
    });

    let cpu_latest = use_api_with_options(
        move || {
            let _ = poll();
            let client = ApiClient::new();
            async move { client.fetch_cpu_latest().await }
        },
        refresh,
    );

    let cpu_history = use_api_with_options(
        move || {
            let _ = poll();
            let client = ApiClient::new();
            async move { client.fetch_cpu_history(60).await }
        },
        refresh,
    );

    let cpu_threads = use_api_with_options(
        move || {
            let _ = poll();
            let client = ApiClient::new();
            async move { client.fetch_cpu_top_threads(15).await }
        },
        refresh,
    );

    let gpu_devices = use_api_with_options(
        move || {
            let _ = poll();
            let client = ApiClient::new();
            async move { client.fetch_gpu_devices().await }
        },
        refresh,
    );

    let gpu_latest = use_api_with_options(
        move || {
            let _ = poll();
            let client = ApiClient::new();
            async move { client.fetch_gpu_latest().await }
        },
        refresh,
    );

    let gpu_history = use_api_with_options(
        move || {
            let _ = poll();
            let client = ApiClient::new();
            async move { client.fetch_gpu_history(60).await }
        },
        refresh,
    );

    let show_gpu = gpu_has_data(&gpu_devices, &gpu_latest);

    rsx! {
        PageContainer {
            PageTitle {
                title: "Dashboard".to_string(),
                subtitle: Some(if show_gpu {
                    "Live CPU and GPU utilization, memory, and process overview".to_string()
                } else {
                    "Live CPU time (user / kernel) and process overview".to_string()
                }),
                icon: Some(&icondata::AiLineChartOutlined),
                header_right: Some(rsx! {
                    PollStatusBar {
                        interval_secs: CPU_POLL_MS / 1000,
                        poll_tick,
                    }
                }),
            }
            {cpu_section(&cpu_latest, &cpu_history, &cpu_threads)}
            if show_gpu {
                {gpu_section(&gpu_devices, &gpu_latest, &gpu_history)}
            }
            {process_section(&overview)}
        }
    }
}

fn gpu_has_data(
    devices: &crate::hooks::ApiState<Vec<GpuDeviceRow>>,
    latest: &crate::hooks::ApiState<Vec<GpuSnapshot>>,
) -> bool {
    if let Some(Ok(devs)) = devices.data.read().as_ref() {
        if !devs.is_empty() {
            return true;
        }
    }
    if let Some(Ok(snaps)) = latest.data.read().as_ref() {
        if !snaps.is_empty() {
            return true;
        }
    }
    false
}

fn cpu_section(
    latest: &crate::hooks::ApiState<Option<CpuSnapshot>>,
    history: &crate::hooks::ApiState<Vec<CpuHistorySample>>,
    threads: &crate::hooks::ApiState<Vec<CpuThreadRow>>,
) -> Element {
    rsx! {
        div { class: "space-y-4 mb-6",
            {cpu_summary_row(latest)}
            Card {
                title: "CPU Time Trend (per sample)",
                content_class: Some("p-4"),
                {cpu_trend_panel(latest, history)}
            }
            Card {
                title: "Top CPU Threads",
                content_class: Some("p-4"),
                {cpu_threads_panel(threads)}
            }
        }
    }
}

fn cpu_summary_row(state: &crate::hooks::ApiState<Option<CpuSnapshot>>) -> Element {
    if state.is_loading() {
        return rsx! {
            div { class: "grid grid-cols-2 sm:grid-cols-3 lg:grid-cols-5 gap-4",
                for _ in 0..5 {
                    div { class: "bg-white border border-gray-200 rounded-lg px-5 py-4 h-24 animate-pulse" }
                }
            }
        };
    }

    match state.data.read().as_ref() {
        Some(Ok(Some(snap))) => rsx! {
            div { class: "grid grid-cols-2 sm:grid-cols-3 lg:grid-cols-5 gap-4",
                StatCard {
                    label: "User CPU",
                    value: format_cpu_ms(snap.delta_user_ns),
                    hint: Some(format!("{} · {}", format_pct(snap.cpu_user_pct), snap.platform)),
                }
                StatCard {
                    label: "Kernel CPU",
                    value: format_cpu_ms(snap.delta_sys_ns),
                    hint: Some(format_pct(snap.cpu_sys_pct)),
                }
                StatCard {
                    label: "Total CPU",
                    value: format_cpu_ms(snap.delta_total_ns),
                    hint: Some(format_pct(snap.cpu_total_pct)),
                }
                StatCard {
                    label: "Memory (RSS)",
                    value: format_rss(snap.rss_kb),
                    hint: Some(format!("{} threads", snap.thread_count)),
                }
                StatCard {
                    label: "Context Switches",
                    value: format!("{}", snap.delta_vol_ctxt + snap.delta_invol_ctxt),
                    hint: Some(format!(
                        "vol {} · invol {}",
                        snap.delta_vol_ctxt, snap.delta_invol_ctxt
                    )),
                }
            }
        },
        Some(Ok(None)) => rsx! {
            div {
                class: "bg-white border border-dashed border-gray-300 rounded-lg px-5 py-6 text-center text-sm text-gray-500",
                "Collecting CPU samples… refresh in a few seconds."
            }
        },
        Some(Err(e)) => rsx! {
            Card {
                title: "CPU Metrics",
                ErrorState { error: e.display_message(), title: Some("Failed to load CPU metrics".to_string()) }
            }
        },
        _ => rsx! { div {} },
    }
}

fn cpu_trend_panel(
    latest: &crate::hooks::ApiState<Option<CpuSnapshot>>,
    history: &crate::hooks::ApiState<Vec<CpuHistorySample>>,
) -> Element {
    if history.is_loading() && latest.is_loading() {
        return rsx! { LoadingState { message: Some("Loading CPU history…".to_string()) } };
    }

    if let Some(Err(e)) = history.data.read().as_ref() {
        return rsx! {
            ErrorState { error: e.display_message(), title: None }
        };
    }

    let samples = history
        .data
        .read()
        .as_ref()
        .and_then(|r| r.as_ref().ok())
        .cloned()
        .unwrap_or_default();

    if samples.is_empty() {
        return rsx! {
            EmptyState {
                message: "No CPU history yet. Sampling runs every second once the collector starts.".to_string()
            }
        };
    }

    let current = latest
        .data
        .read()
        .as_ref()
        .and_then(|r| r.as_ref().ok())
        .and_then(|o| o.clone())
        .map(|s| {
            format!(
                "{} user · {} sys",
                format_cpu_ms(s.delta_user_ns),
                format_cpu_ms(s.delta_sys_ns)
            )
        })
        .unwrap_or_else(|| "—".to_string());

    rsx! {
        div { class: "space-y-3",
            div { class: "flex items-baseline justify-between gap-4 flex-wrap",
                span { class: "text-sm text-gray-500", "Latest sample" }
                span { class: "text-sm font-semibold text-gray-900", "{current}" }
            }
            div { class: "flex items-center gap-4 text-xs text-gray-500",
                span { class: "inline-flex items-center gap-1",
                    span { class: "w-2.5 h-2.5 rounded-sm bg-blue-500" }
                    "User"
                }
                span { class: "inline-flex items-center gap-1",
                    span { class: "w-2.5 h-2.5 rounded-sm bg-amber-500" }
                    "Kernel"
                }
            }
            CpuTimeSparkline { samples }
            p { class: "text-xs text-gray-400", "Updates every {CPU_POLL_MS / 1000}s · samples every 1s · values are CPU time per interval" }
        }
    }
}

fn cpu_threads_panel(state: &crate::hooks::ApiState<Vec<CpuThreadRow>>) -> Element {
    if state.is_loading() {
        return rsx! { LoadingState { message: Some("Loading thread CPU…".to_string()) } };
    }
    match state.data.read().as_ref() {
        Some(Ok(rows)) if !rows.is_empty() => {
            rsx! {
                div { class: "space-y-3",
                    p { class: "text-xs text-gray-500",
                        "Ranked by CPU time in the latest sample. Use Stack / Traces / Profile to investigate."
                    }
                    CpuThreadsTable { threads: rows.clone() }
                }
            }
        }
        Some(Err(e)) => rsx! {
            ErrorState { error: e.display_message(), title: None }
        },
        _ => rsx! {
            EmptyState {
                message: "No thread-level CPU data yet — wait for a few sampling intervals.".to_string()
            }
        },
    }
}

#[component]
fn CpuTimeSparkline(samples: Vec<CpuHistorySample>) -> Element {
    let max = samples
        .iter()
        .map(|s| s.total_ms)
        .fold(0.0f32, f32::max)
        .max(1.0);

    rsx! {
        div {
            class: "flex items-end gap-0.5 h-20 w-full",
            for (i, s) in samples.iter().enumerate() {
                div {
                    key: "{i}",
                    class: "flex-1 flex flex-col justify-end min-w-[3px] h-full gap-px",
                    title: format!("user {:.1} ms · sys {:.1} ms", s.user_ms, s.sys_ms),
                    div {
                        class: "w-full rounded-sm bg-amber-500/85 hover:bg-amber-600 transition-colors",
                        style: "height: {(s.sys_ms / max * 100.0).max(0.0)}%",
                    }
                    div {
                        class: "w-full rounded-sm bg-blue-500/85 hover:bg-blue-600 transition-colors",
                        style: "height: {(s.user_ms / max * 100.0).max(0.0)}%",
                    }
                }
            }
        }
    }
}

fn gpu_section(
    devices: &crate::hooks::ApiState<Vec<GpuDeviceRow>>,
    latest: &crate::hooks::ApiState<Vec<GpuSnapshot>>,
    history: &crate::hooks::ApiState<HashMap<i32, Vec<GpuHistorySample>>>,
) -> Element {
    rsx! {
        div { class: "space-y-4 mb-6",
            Card {
                title: "GPU",
                content_class: Some("p-4"),
                {gpu_devices_panel(devices, latest, history)}
            }
        }
    }
}

/// Adaptive grid columns from device count (1 → full width, 8 → wraps in 4 cols).
fn gpu_grid_class(device_count: usize) -> &'static str {
    match device_count {
        0 | 1 => "grid-cols-1",
        2 => "grid-cols-1 md:grid-cols-2",
        3 => "grid-cols-1 md:grid-cols-2 xl:grid-cols-3",
        4 => "grid-cols-1 sm:grid-cols-2 xl:grid-cols-4",
        _ => "grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 2xl:grid-cols-4",
    }
}

fn gpu_util_pct_value(snap: &GpuSnapshot) -> f32 {
    snap.gpu_util_pct.unwrap_or(0.0).clamp(0.0, 100.0)
}

fn gpu_util_hint(snap: &GpuSnapshot) -> String {
    if snap.backend == "metal" {
        format!(
            "renderer {} · tiler {}",
            format_opt_pct(snap.renderer_util_pct),
            format_opt_pct(snap.tiler_util_pct)
        )
    } else {
        format!("mem ctrl {}", format_opt_pct(snap.mem_controller_util_pct))
    }
}

fn gpu_devices_panel(
    devices: &crate::hooks::ApiState<Vec<GpuDeviceRow>>,
    latest: &crate::hooks::ApiState<Vec<GpuSnapshot>>,
    history: &crate::hooks::ApiState<HashMap<i32, Vec<GpuHistorySample>>>,
) -> Element {
    if devices.is_loading() && latest.is_loading() {
        return rsx! { LoadingState { message: Some("Loading GPU metrics…".to_string()) } };
    }

    if let Some(Err(e)) = devices.data.read().as_ref() {
        return rsx! {
            ErrorState { error: e.display_message(), title: Some("Failed to load GPU devices".to_string()) }
        };
    }
    if let Some(Err(e)) = latest.data.read().as_ref() {
        return rsx! {
            ErrorState { error: e.display_message(), title: Some("Failed to load GPU utilization".to_string()) }
        };
    }

    let latest_map: HashMap<i32, GpuSnapshot> = latest
        .data
        .read()
        .as_ref()
        .and_then(|r| r.as_ref().ok())
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|s| (s.device_id, s))
        .collect();

    let device_rows: Vec<GpuDeviceRow> = devices
        .data
        .read()
        .as_ref()
        .and_then(|r| r.as_ref().ok())
        .cloned()
        .unwrap_or_default();

    let mut snapshots: Vec<GpuSnapshot> = latest_map.into_values().collect();
    snapshots.sort_by_key(|s| s.device_id);

    if device_rows.is_empty() && snapshots.is_empty() {
        return rsx! {
            EmptyState { message: "No GPU devices detected.".to_string() }
        };
    }

    let device_count = snapshots.len().max(device_rows.len());
    let grid_class = gpu_grid_class(device_count);
    let featured = device_count == 1;

    rsx! {
        div { class: "space-y-5",
            if !snapshots.is_empty() {
                if featured {
                    {gpu_hero_panel(&snapshots[0])}
                } else {
                    div {
                        class: "grid {grid_class} gap-4",
                        for snap in snapshots.iter() {
                            {gpu_device_card(snap)}
                        }
                    }
                }
            }
            {gpu_trend_panel(&snapshots, history, featured, grid_class)}
            if !featured || device_rows.len() > 1 {
                {gpu_devices_table(&device_rows)}
            }
        }
    }
}

fn gpu_hero_panel(snap: &GpuSnapshot) -> Element {
    let util_hint = gpu_util_hint(snap);
    let gpu_pct = gpu_util_pct_value(snap);
    let mem_hint = format!(
        "{} / {} used",
        format_bytes(snap.used_bytes),
        format_bytes(snap.total_bytes)
    );

    rsx! {
        div { class: "rounded-xl border border-gray-200 bg-white shadow-sm overflow-hidden",
            div { class: "bg-gradient-to-r from-violet-50 via-white to-emerald-50 px-6 py-5 border-b border-gray-100",
                div { class: "flex flex-wrap items-start justify-between gap-4",
                    div { class: "min-w-0 flex-1",
                        p { class: "text-xs font-medium uppercase tracking-wide text-gray-500",
                            "Device"
                        }
                        h3 { class: "text-xl font-semibold text-gray-900 mt-1 truncate",
                            "{snap.name}"
                        }
                        p { class: "text-sm text-gray-500 mt-1",
                            "{snap.backend} · {snap.memory_model}"
                            if let Some(ref chip) = snap.chip {
                                " · {chip}"
                            }
                        }
                    }
                    span { class: "shrink-0 text-xs font-semibold px-3 py-1.5 rounded-full bg-violet-100 text-violet-800",
                        "GPU {snap.device_id}"
                    }
                }
            }
            div { class: "px-6 py-6 grid grid-cols-1 sm:grid-cols-2 xl:grid-cols-4 gap-6",
                GpuHeroMetric {
                    label: "GPU Utilization",
                    value: format_opt_pct(snap.gpu_util_pct),
                    pct: gpu_pct,
                    bar_class: "bg-violet-500",
                    value_class: "text-violet-700",
                    hint: Some(util_hint),
                }
                GpuHeroMetric {
                    label: "Memory Used",
                    value: format_pct(snap.mem_used_pct),
                    pct: snap.mem_used_pct,
                    bar_class: "bg-emerald-500",
                    value_class: "text-emerald-700",
                    hint: Some(mem_hint),
                }
                div { class: "sm:col-span-2 xl:col-span-2 flex flex-col justify-center gap-3",
                    div { class: "grid grid-cols-2 gap-4",
                        div {
                            p { class: "text-xs font-medium text-gray-500 uppercase tracking-wide", "Free" }
                            p { class: "text-lg font-semibold text-gray-900 mt-1", "{format_bytes(snap.free_bytes)}" }
                        }
                        div {
                            p { class: "text-xs font-medium text-gray-500 uppercase tracking-wide", "Total" }
                            p { class: "text-lg font-semibold text-gray-900 mt-1", "{format_bytes(snap.total_bytes)}" }
                        }
                    }
                    GpuProgressBar { pct: snap.mem_used_pct, bar_class: "bg-emerald-500" }
                }
            }
        }
    }
}

#[component]
fn GpuHeroMetric(
    label: &'static str,
    value: String,
    pct: f32,
    bar_class: &'static str,
    value_class: &'static str,
    #[props(optional)] hint: Option<String>,
) -> Element {
    rsx! {
        div { class: "space-y-3",
            p { class: "text-xs font-medium text-gray-500 uppercase tracking-wide", "{label}" }
            p { class: "text-3xl font-bold {value_class}", "{value}" }
            GpuProgressBar { pct, bar_class }
            if let Some(h) = hint {
                p { class: "text-xs text-gray-500", "{h}" }
            }
        }
    }
}

#[component]
fn GpuProgressBar(pct: f32, bar_class: &'static str) -> Element {
    let width = pct.clamp(0.0, 100.0);
    rsx! {
        div { class: "h-2.5 rounded-full bg-gray-100 overflow-hidden",
            div {
                class: "h-full rounded-full transition-all duration-300 {bar_class}",
                style: "width: {width}%",
            }
        }
    }
}

fn gpu_device_card(snap: &GpuSnapshot) -> Element {
    let util_hint = gpu_util_hint(snap);
    let gpu_pct = gpu_util_pct_value(snap);

    rsx! {
        div { class: "bg-white border border-gray-200 rounded-xl px-5 py-4 shadow-sm space-y-4",
            div { class: "flex items-start justify-between gap-2",
                div { class: "min-w-0",
                    div { class: "text-xs font-medium text-gray-500 uppercase tracking-wide truncate",
                        "{gpu_device_label(snap.device_id, &snap.name)}"
                    }
                    if let Some(ref chip) = snap.chip {
                        div { class: "text-xs text-gray-400 mt-0.5 truncate", "{chip}" }
                    }
                }
                span { class: "shrink-0 text-[10px] font-medium px-2 py-0.5 rounded bg-gray-100 text-gray-600",
                    "{snap.backend}"
                }
            }
            div { class: "space-y-3",
                div {
                    div { class: "flex items-baseline justify-between gap-2 mb-1.5",
                        span { class: "text-xs text-gray-500", "GPU Util" }
                        span { class: "text-xl font-bold text-violet-700",
                            "{format_opt_pct(snap.gpu_util_pct)}"
                        }
                    }
                    GpuProgressBar { pct: gpu_pct, bar_class: "bg-violet-500" }
                }
                div {
                    div { class: "flex items-baseline justify-between gap-2 mb-1.5",
                        span { class: "text-xs text-gray-500", "Memory" }
                        span { class: "text-xl font-bold text-emerald-700",
                            "{format_pct(snap.mem_used_pct)}"
                        }
                    }
                    GpuProgressBar { pct: snap.mem_used_pct, bar_class: "bg-emerald-500" }
                }
            }
            div { class: "text-xs text-gray-500 pt-1 border-t border-gray-100",
                "{format_bytes(snap.used_bytes)} / {format_bytes(snap.total_bytes)} · {util_hint}"
            }
        }
    }
}

fn gpu_devices_table(devices: &[GpuDeviceRow]) -> Element {
    if devices.is_empty() {
        return rsx! { div {} };
    }
    rsx! {
        div { class: "overflow-x-auto border border-gray-200 rounded-lg",
            table { class: "min-w-full text-sm",
                thead { class: "bg-gray-50 text-left text-xs uppercase text-gray-500",
                    tr {
                        th { class: "px-3 py-2", "ID" }
                        th { class: "px-3 py-2", "Name" }
                        th { class: "px-3 py-2", "Backend" }
                        th { class: "px-3 py-2", "Memory" }
                        th { class: "px-3 py-2", "Chip / CC" }
                    }
                }
                tbody {
                    for d in devices {
                        tr { class: "border-t border-gray-100",
                            td { class: "px-3 py-2 font-mono", "{d.device_id}" }
                            td { class: "px-3 py-2", "{d.name}" }
                            td { class: "px-3 py-2 text-gray-600", "{d.backend}" }
                            td { class: "px-3 py-2", "{format_bytes(d.total_mem_bytes)}" }
                            td { class: "px-3 py-2 text-gray-600",
                                {d.chip.clone().or(d.compute_capability.clone()).unwrap_or_else(|| "—".to_string())}
                            }
                        }
                    }
                }
            }
        }
    }
}

fn gpu_trend_panel(
    snapshots: &[GpuSnapshot],
    history: &crate::hooks::ApiState<HashMap<i32, Vec<GpuHistorySample>>>,
    featured: bool,
    grid_class: &str,
) -> Element {
    if history.is_loading() && snapshots.is_empty() {
        return rsx! { LoadingState { message: Some("Loading GPU history…".to_string()) } };
    }

    if let Some(Err(e)) = history.data.read().as_ref() {
        return rsx! { ErrorState { error: e.display_message(), title: None } };
    }

    let history_map = history
        .data
        .read()
        .as_ref()
        .and_then(|r| r.as_ref().ok())
        .cloned()
        .unwrap_or_default();

    if history_map.is_empty() {
        return rsx! {
            EmptyState {
                message: "Collecting GPU samples… refresh in a few seconds.".to_string()
            }
        };
    }

    let mut device_ids: Vec<i32> = history_map.keys().copied().collect();
    device_ids.sort();

    let names: HashMap<i32, String> = snapshots
        .iter()
        .map(|s| (s.device_id, s.name.clone()))
        .collect();

    let sparkline_height = if featured { "h-32" } else { "h-20" };
    let trend_title = if featured {
        "Utilization trend"
    } else {
        "Utilization trend (per device)"
    };

    rsx! {
        div {
            class: if featured {
                "rounded-xl border border-gray-200 bg-gray-50/80 p-5 space-y-4"
            } else {
                "space-y-4 pt-2 border-t border-gray-100"
            },
            div { class: "flex flex-wrap items-center justify-between gap-3",
                p {
                    class: if featured { "text-sm font-semibold text-gray-800" } else { "text-sm font-medium text-gray-700" },
                    "{trend_title}"
                }
                div { class: "flex items-center gap-4 text-xs text-gray-500",
                    span { class: "inline-flex items-center gap-1.5",
                        span { class: "w-2.5 h-2.5 rounded-sm bg-violet-500" }
                        "GPU util %"
                    }
                    span { class: "inline-flex items-center gap-1.5",
                        span { class: "w-2.5 h-2.5 rounded-sm bg-emerald-500" }
                        "Memory used %"
                    }
                }
            }
            div {
                class: "grid {grid_class} gap-4",
                for id in device_ids {
                    {
                        let samples = history_map.get(&id).cloned().unwrap_or_default();
                        let label = names
                            .get(&id)
                            .map(|n| gpu_device_label(id, n))
                            .unwrap_or_else(|| format!("GPU {id}"));
                        rsx! {
                            div {
                                class: if featured {
                                    "bg-white border border-gray-200 rounded-lg p-4 space-y-3 shadow-sm"
                                } else {
                                    "bg-white border border-gray-100 rounded-lg p-3 space-y-2"
                                },
                                if !featured {
                                    div { class: "text-xs font-medium text-gray-700", "{label}" }
                                }
                                GpuUtilSparkline { samples: samples.clone(), height_class: sparkline_height }
                            }
                        }
                    }
                }
            }
            p { class: "text-xs text-gray-400",
                "Updates every {CPU_POLL_MS / 1000}s · one series per device"
            }
        }
    }
}

#[component]
fn GpuUtilSparkline(samples: Vec<GpuHistorySample>, height_class: &'static str) -> Element {
    let max = samples
        .iter()
        .map(|s| s.mem_used_pct.max(s.gpu_util_pct))
        .fold(0.0f32, f32::max)
        .max(1.0);

    rsx! {
        div {
            class: "flex items-end gap-0.5 w-full {height_class}",
            for (i, s) in samples.iter().enumerate() {
                div {
                    key: "{i}",
                    class: "flex-1 flex flex-col justify-end min-w-[3px] h-full gap-px",
                    title: format!("gpu {:.0}% · mem {:.0}%", s.gpu_util_pct, s.mem_used_pct),
                    div {
                        class: "w-full rounded-sm bg-emerald-500/85 hover:bg-emerald-600 transition-colors",
                        style: "height: {(s.mem_used_pct / max * 100.0).max(0.0)}%",
                    }
                    div {
                        class: "w-full rounded-sm bg-violet-500/85 hover:bg-violet-600 transition-colors",
                        style: "height: {(s.gpu_util_pct / max * 100.0).max(0.0)}%",
                    }
                }
            }
        }
    }
}

fn process_section(state: &crate::hooks::ApiState<Process>) -> Element {
    if state.is_loading() {
        return rsx! {
            Card {
                title: "Process Information",
                LoadingState { message: Some("Loading process information…".to_string()) }
            }
        };
    }
    let data = state.data.read();
    if let Some(Err(err)) = data.as_ref() {
        return rsx! {
            Card {
                title: "Process Information",
                ErrorState { error: err.display_message(), title: None }
            }
        };
    }
    let Some(Ok(process)) = data.as_ref() else {
        return rsx! { div {} };
    };
    rsx! {
        div { class: "space-y-4",
            ProcessContextSync { process: process.clone() }
            Card {
                title: "Process Information",
                KeyValueList {
                    items: vec![
                        ("Process ID (PID):", process.pid.to_string()),
                        ("Executable Path:", process.exe.clone()),
                        ("Command Line:", process.cmd.clone()),
                        ("Working Directory:", process.cwd.clone()),
                    ]
                }
            }
            Card {
                title: "Threads Information",
                div {
                    class: "space-y-3",
                    div { class: "text-sm text-gray-600", "Total threads: {process.threads.len()}" }
                    ThreadsPreview {
                        threads: process.threads.clone(),
                        pid: process.pid,
                    }
                }
            }
            Card {
                title: "Environment Variables",
                EnvVars { env: process.env.clone() }
            }
        }
    }
}

#[component]
fn ThreadsPreview(threads: Vec<u64>, pid: i32) -> Element {
    let navigator = use_navigator();
    let mut expanded = use_signal(|| false);
    let total = threads.len();
    let limit = if expanded() { total } else { THREADS_PREVIEW.min(total) };
    let hidden = total.saturating_sub(limit);

    rsx! {
        div { class: "space-y-2",
            div { class: "flex flex-wrap gap-2",
                for tid in threads.iter().take(limit) {
                    {
                        let tid_i32 = *tid as i32;
                        let tid_str = tid.to_string();
                        rsx! {
                            button {
                                class: "px-3 py-1 text-sm bg-blue-100 text-blue-800 hover:bg-blue-200 rounded-md transition-colors font-mono",
                                title: "Open stack for this thread and set investigation context",
                                onclick: move |_| {
                                    crate::state::investigation::set_thread_context(
                                        tid_i32,
                                        None,
                                        Some(pid),
                                    );
                                    navigator.push(Route::StackWithTidPage {
                                        tid: tid_str.clone(),
                                    });
                                },
                                "{tid}"
                            }
                        }
                    }
                }
            }
            if hidden > 0 {
                button {
                    class: "text-sm text-blue-600 hover:underline",
                    onclick: move |_| expanded.set(true),
                    "Show {hidden} more threads…"
                }
            } else if expanded() && total > THREADS_PREVIEW {
                button {
                    class: "text-sm text-blue-600 hover:underline",
                    onclick: move |_| expanded.set(false),
                    "Show less"
                }
            }
        }
    }
}

#[component]
fn ProcessContextSync(process: Process) -> Element {
    let pid = process.pid;
    let exe = process.exe.clone();
    use_effect(move || {
        sync_overview_process_context(pid, &exe);
    });
    rsx! {}
}

#[component]
fn EnvVars(env: std::collections::HashMap<String, String>) -> Element {
    let mut expanded = use_signal(|| false);
    let total = env.len();
    let show_all = expanded();
    let preview_limit = if show_all { total } else { ENV_VARS_PREVIEW.min(total) };
    let mut entries: Vec<_> = env.into_iter().collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let hidden = total.saturating_sub(preview_limit);

    rsx! {
        div {
            class: "space-y-3",
            div { class: "text-sm text-gray-600", "Total environment variables: {total}" }
            div {
                class: "space-y-2",
                for (name, value) in entries.into_iter().take(preview_limit) {
                    div {
                        class: "flex justify-between items-start py-2 border-b border-gray-200 last:border-b-0",
                        span { class: "font-medium text-gray-700 font-mono text-sm shrink-0 mr-4", "{name}" }
                        span { class: "font-mono text-sm bg-gray-100 text-gray-900 px-3 py-1.5 rounded break-all text-right", "{value}" }
                    }
                }
            }
            if hidden > 0 {
                button {
                    class: "text-sm text-blue-600 hover:underline",
                    onclick: move |_| expanded.set(true),
                    "Show {hidden} more…"
                }
            } else if show_all && total > ENV_VARS_PREVIEW {
                button {
                    class: "text-sm text-blue-600 hover:underline",
                    onclick: move |_| expanded.set(false),
                    "Show less"
                }
            }
        }
    }
}
