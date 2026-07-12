//! TorchProbe overhead dashboard panel (presentation only).

use dioxus::prelude::*;
use probing_proto::prelude::DataFrame;

use crate::api::{empty_dataframe, is_nccl_counters_missing, ApiClient};
use crate::components::common::{AppErrorDisplay, EmptyState, LoadingState};
use crate::components::stat_card::StatCard;
use crate::hooks::{use_api_with_options, ApiFetchOptions, ApiState};
use crate::overhead::sql::WINDOW_STEPS;
use crate::overhead::{
    dataframe_rows, format_pct_display, format_pct_signed, format_step_ms, parse_overhead_steps,
    table_missing_message, OverheadLevel, OverheadSnapshot, OverheadStep,
};
use crate::utils::error::AppError;

fn refresh_options() -> ApiFetchOptions {
    ApiFetchOptions {
        keep_previous_while_refreshing: true,
    }
}

#[component]
pub fn TorchOverheadPanel(refresh_tick: u32) -> Element {
    let refresh = refresh_options();
    let nccl_skip = use_signal(|| false);

    let summary = use_api_with_options(
        move || {
            let _ = refresh_tick;
            async move { ApiClient::new().fetch_overhead_summary().await }
        },
        refresh,
    );
    let recent = use_api_with_options(
        move || {
            let _ = refresh_tick;
            async move { ApiClient::new().fetch_overhead_recent_steps().await }
        },
        refresh,
    );
    let train_step = use_api_with_options(
        move || {
            let _ = refresh_tick;
            async move { ApiClient::new().fetch_overhead_train_step_median().await }
        },
        refresh,
    );
    let nccl = use_api_with_options(
        move || {
            let _ = refresh_tick;
            let mut nccl_skip = nccl_skip;
            let skip = nccl_skip();
            async move {
                if skip {
                    return Ok(empty_dataframe());
                }
                match ApiClient::new().fetch_overhead_nccl_counters().await {
                    Ok(None) => {
                        nccl_skip.set(true);
                        Ok(empty_dataframe())
                    }
                    Ok(Some(df)) => Ok(df),
                    Err(e) if is_nccl_counters_missing(&e) => {
                        nccl_skip.set(true);
                        Ok(empty_dataframe())
                    }
                    Err(e) => Err(e),
                }
            }
        },
        refresh,
    );

    let refreshing = summary.is_loading() || recent.is_loading() || train_step.is_loading();

    if summary.data.read().is_none() && summary.is_loading() {
        return rsx! {
            LoadingState { message: "Loading overhead…".to_string() }
        };
    }

    overhead_body(
        &resolved_result(&summary),
        &resolved_result(&recent),
        &resolved_result(&train_step),
        &resolved_result(&nccl),
        refreshing,
        refresh_tick,
    )
}

fn resolved_result<T: Clone>(state: &ApiState<T>) -> Result<T, AppError> {
    state
        .data
        .read()
        .clone()
        .unwrap_or_else(|| Err(AppError::Api("overhead data not loaded".into())))
}

fn train_step_ms(train_step: &Result<DataFrame, AppError>) -> Option<f64> {
    train_step
        .as_ref()
        .ok()
        .and_then(|df| crate::overhead::df_scalar_f64(df, "train_step_median_ms", 0))
}

#[component]
fn OverheadAlerts(snap: OverheadSnapshot) -> Element {
    if !snap.shadow_enabled() {
        return rsx! {
            div {
                class: "rounded-md border border-slate-300 bg-slate-50 px-3 py-2 text-xs text-slate-800",
                "Shadow baseline is disabled ("
                code { class: "font-mono", "shadow=off" }
                "). Set "
                code { class: "font-mono", "probing.torch.profiling=on,shadow=4:1" }
                " to compare probed vs baseline steps."
            }
        };
    }
    if snap.shadow_n > 0 && snap.shadow_n < 5 {
        return rsx! {
            div { class: "rounded-md border border-amber-300 bg-amber-50 px-3 py-2 text-xs text-amber-900",
                "Only {snap.shadow_n} shadow step(s) in the window — percentages need more samples."
            }
        };
    }
    if snap.shadow_enabled() && snap.shadow_n == 0 {
        return rsx! {
            div { class: "rounded-md border border-blue-200 bg-blue-50 px-3 py-2 text-xs text-blue-900",
                "Collecting shadow baseline steps — wait for one full shadow cadence cycle."
            }
        };
    }
    rsx! { div {} }
}

#[component]
fn OverheadVerdict(snap: OverheadSnapshot) -> Element {
    if !snap.is_stable() {
        return rsx! { div {} };
    }
    let Some(pct) = snap.dispatch_overhead_pct else {
        return rsx! { div {} };
    };
    let level = snap.dispatch_level();
    let display = format_pct_signed(pct);
    match level {
        OverheadLevel::Low => rsx! {
            div {
                class: "rounded-md border border-emerald-200 bg-emerald-50 px-3 py-2.5 text-sm text-emerald-900",
                span { class: "font-medium", "Profiler overhead {display}" }
                " — adds about {display} per step vs shadow baseline. Typical for default settings (<5%)."
            }
        },
        OverheadLevel::Moderate => rsx! {
            div {
                class: "rounded-md border border-amber-200 bg-amber-50 px-3 py-2.5 text-sm text-amber-900",
                span { class: "font-medium", "Profiler overhead {display}" }
                " — above the usual <5% band. Try a lower sample rate or confirm shadow 4:1 is enabled."
            }
        },
        OverheadLevel::High => rsx! {
            div {
                class: "rounded-md border border-orange-200 bg-orange-50 px-3 py-2.5 text-sm text-orange-900",
                span { class: "font-medium", "Profiler overhead {display}" }
                " — unusually high. Check trace_spans, sample rate, and GPU defer settings."
            }
        },
        OverheadLevel::Unknown => rsx! { div {} },
    }
}

fn overhead_body(
    summary: &Result<DataFrame, AppError>,
    recent: &Result<DataFrame, AppError>,
    train_step: &Result<DataFrame, AppError>,
    nccl: &Result<DataFrame, AppError>,
    refreshing: bool,
    refresh_tick: u32,
) -> Element {
    if let Err(err) = summary {
        if let Some(message) = table_missing_message(err) {
            return rsx! {
                EmptyState { message: message.to_string() }
            };
        }
        return rsx! {
            AppErrorDisplay { error: err.clone(), title: None }
        };
    }

    let snap = OverheadSnapshot::from_summary(summary.as_ref().expect("checked Ok"))
        .with_train_step_median(train_step_ms(train_step));
    let recent_df = recent.as_ref().ok();
    let row_count = recent_df.map(dataframe_rows).unwrap_or(0);

    if row_count == 0 {
        return rsx! {
            EmptyState {
                message: "No python.torch_step_timing rows yet — enable SET probing.torch.profiling=on and wait for shadow cycles (default 4:1).".to_string()
            }
        };
    }

    if !snap.shadow_enabled() {
        return rsx! {
            div { class: "space-y-3",
                OverheadAlerts { snap: snap.clone() }
                p { class: "text-xs text-gray-500",
                    "Timing rows exist but shadow baseline is off — enable shadow cadence for overhead %."
                }
            }
        };
    }

    let steps = recent_df.map(parse_overhead_steps).unwrap_or_default();
    let meta = format!(
        "Window: {} · cadence {} · sampling {}",
        snap.window_label(),
        snap.cadence_label(),
        snap.sampling_label()
    );

    rsx! {
        div { class: "space-y-5",
            div { class: "flex flex-wrap items-center justify-between gap-2",
                div { class: "space-y-0.5",
                    p { class: "text-sm font-medium text-gray-900",
                        "Step {snap.latest_step} · training & hook cost"
                    }
                    p { class: "text-xs text-gray-500", "{meta}" }
                }
                if refreshing {
                    span {
                        class: "inline-flex items-center gap-1.5 text-[11px] text-emerald-700",
                        span {
                            class: "inline-block w-2.5 h-2.5 border-2 border-emerald-500 border-t-transparent rounded-full animate-spin"
                        }
                        "Updating…"
                    }
                }
            }

            OverheadAlerts { snap: snap.clone() }
            OverheadVerdict { snap: snap.clone() }

            div { class: "grid grid-cols-1 sm:grid-cols-3 gap-3",
                StatCard {
                    label: "Typical overhead".to_string(),
                    value: format_pct_display(snap.dispatch_overhead_pct),
                    hint: Some("Non-sampled steps vs shadow · primary number".to_string()),
                }
                StatCard {
                    label: "Shadow step time".to_string(),
                    value: snap.shadow_median_ms.map(format_step_ms).unwrap_or_else(|| "—".to_string()),
                    hint: Some(format!(
                        "Observed baseline · n={} shadow steps",
                        snap.shadow_n
                    )),
                }
                if let Some(ms) = snap.train_step_median_ms {
                    StatCard {
                        label: "train.step compute".to_string(),
                        value: format_step_ms(ms),
                        hint: Some("Span timing only · excludes hook dispatch".to_string()),
                    }
                } else {
                    StatCard {
                        label: "Probed wall time".to_string(),
                        value: snap.probed_median_ms.map(format_step_ms).unwrap_or_else(|| "—".to_string()),
                        hint: Some(format!("All probed steps · n={}", snap.probed_n)),
                    }
                }
            }

            section { class: "space-y-2",
                SectionTitle {
                    title: "Observed medians".to_string(),
                    subtitle: "Hook-to-hook wall clock (perf_counter) · includes DataLoader wait between steps — matches Progress Time, not compute-only prints".to_string(),
                }
                MetricTable {
                    rows: observed_rows(&snap),
                }
            }

            section { class: "space-y-2",
                SectionTitle {
                    title: "Overhead summary".to_string(),
                    subtitle: "Rounded % for readability · details below".to_string(),
                }
                MetricTable {
                    rows: primary_computed_rows(&snap),
                }
                details { class: "text-sm text-gray-600",
                    summary { class: "cursor-pointer text-xs text-gray-500 hover:text-gray-700",
                        "Advanced breakdown (sampled path, blended median)"
                    }
                    div { class: "mt-2",
                        MetricTable {
                            rows: advanced_computed_rows(&snap),
                        }
                    }
                }
            }

            if !steps.is_empty() {
                section { class: "space-y-2",
                    SectionTitle {
                        title: "Recent steps".to_string(),
                        subtitle: format!("Per-step wall time · last {} bars", WINDOW_STEPS.min(24)),
                    }
                    TorchOverheadTimeline { steps: steps.clone(), refresh_tick: refresh_tick }
                }
            }

            {nccl_footnote(nccl)}
        }
    }
}

#[derive(Clone, PartialEq)]
struct MetricRow {
    name: String,
    value: String,
    detail: String,
}

fn observed_rows(snap: &OverheadSnapshot) -> Vec<MetricRow> {
    vec![
        MetricRow {
            name: "Shadow baseline".to_string(),
            value: snap
                .shadow_median_ms
                .map(format_step_ms)
                .unwrap_or_else(|| "—".to_string()),
            detail: format!("n = {} · hooks bypassed", snap.shadow_n),
        },
        MetricRow {
            name: "Probed · dispatch path".to_string(),
            value: snap
                .dispatch_median_ms
                .map(format_step_ms)
                .unwrap_or_else(|| "—".to_string()),
            detail: format!("n = {} · sampled = 0", snap.dispatch_n),
        },
        MetricRow {
            name: "Probed · sampled path".to_string(),
            value: snap
                .sampled_median_ms
                .map(format_step_ms)
                .unwrap_or_else(|| "—".to_string()),
            detail: format!("n = {} · includes trace flush", snap.sampled_n),
        },
        MetricRow {
            name: "Probed · all".to_string(),
            value: snap
                .probed_median_ms
                .map(format_step_ms)
                .unwrap_or_else(|| "—".to_string()),
            detail: format!("n = {} · dispatch + sampled mix", snap.probed_n),
        },
    ]
}

fn primary_computed_rows(snap: &OverheadSnapshot) -> Vec<MetricRow> {
    vec![
        MetricRow {
            name: "Typical overhead".to_string(),
            value: format_pct_display(snap.dispatch_overhead_pct),
            detail: "Most steps (non-sampled) vs shadow baseline".to_string(),
        },
        MetricRow {
            name: "Effective overhead".to_string(),
            value: format_pct_display(snap.amortized_overhead_pct),
            detail: amortized_detail(snap),
        },
    ]
}

fn advanced_computed_rows(snap: &OverheadSnapshot) -> Vec<MetricRow> {
    vec![
        MetricRow {
            name: "Sampled path".to_string(),
            value: format_pct_display(snap.sampled_overhead_pct),
            detail: "Heavy path (trace flush) · tuning sample rate only".to_string(),
        },
        MetricRow {
            name: "Blended median".to_string(),
            value: format_pct_display(snap.blended_overhead_pct),
            detail: "All probed steps mixed · informational".to_string(),
        },
    ]
}

fn amortized_detail(snap: &OverheadSnapshot) -> String {
    let pct = |rate: f64| format!("{:.0}%", rate * 100.0);
    match (
        snap.sample_rate.filter(|r| r.is_finite()),
        snap.sampled_n > 0,
    ) {
        (Some(rate), true) => format!("(1 − {}) × dispatch + {} × sampled", pct(rate), pct(rate)),
        (_, true) if snap.probed_n > 0 => {
            let rate = snap.sampled_n as f64 / snap.probed_n as f64;
            format!("(1 − {}) × dispatch + {} × sampled", pct(rate), pct(rate))
        }
        _ => "no sampling · same as typical overhead".to_string(),
    }
}

#[component]
fn SectionTitle(title: String, subtitle: String) -> Element {
    rsx! {
        div {
            p { class: "text-sm font-semibold text-gray-900", "{title}" }
            p { class: "text-xs text-gray-500 mt-0.5", "{subtitle}" }
        }
    }
}

#[component]
fn MetricTable(rows: Vec<MetricRow>) -> Element {
    rsx! {
        div { class: "overflow-x-auto rounded-lg border border-gray-200",
            table { class: "min-w-full text-sm",
                thead {
                    tr { class: "bg-gray-50 text-left text-xs uppercase tracking-wide text-gray-500",
                        th { class: "px-3 py-2 font-medium", "Metric" }
                        th { class: "px-3 py-2 font-medium text-right", "Value" }
                        th { class: "px-3 py-2 font-medium", "Notes" }
                    }
                }
                tbody { class: "divide-y divide-gray-100",
                    for (i, row) in rows.iter().enumerate() {
                        tr { key: "{i}", class: "bg-white",
                            td { class: "px-3 py-2.5 text-gray-800 font-medium", "{row.name}" }
                            td { class: "px-3 py-2.5 text-right tabular-nums font-semibold text-gray-900",
                                "{row.value}"
                            }
                            td { class: "px-3 py-2.5 text-gray-500 text-xs", "{row.detail}" }
                        }
                    }
                }
            }
        }
    }
}

fn nccl_footnote(nccl: &Result<DataFrame, AppError>) -> Element {
    match nccl {
        Ok(df) if dataframe_rows(df) > 0 => {
            let pool_exhausted =
                crate::overhead::df_scalar_i64(df, "pool_exhausted", 0).unwrap_or(0);
            let write_errors = crate::overhead::df_scalar_i64(df, "write_errors", 0).unwrap_or(0);
            if pool_exhausted > 0 || write_errors > 0 {
                rsx! {
                    p { class: "text-xs text-amber-800 bg-amber-50 border border-amber-200 rounded-md px-3 py-2",
                        "NCCL profiler health: pool_exhausted={pool_exhausted}, write_errors={write_errors}. "
                        "NCCL has no in-run shadow baseline — offline: "
                        code { class: "font-mono", "examples/run_nccl_profiler_bench.sh" }
                    }
                }
            } else {
                rsx! {
                    p { class: "text-xs text-gray-500",
                        "NCCL profiler active (counters OK). No in-run NCCL overhead % — offline bench available."
                    }
                }
            }
        }
        _ => rsx! {
            p { class: "text-xs text-gray-500",
                "NCCL profiler inactive — metrics above are TorchProbe hook overhead only."
            }
        },
    }
}

#[component]
fn TorchOverheadTimeline(steps: Vec<OverheadStep>, refresh_tick: u32) -> Element {
    let max_ms = steps
        .iter()
        .map(|s| s.duration_ms)
        .fold(0.0f64, f64::max)
        .max(1.0);

    rsx! {
        div { class: "space-y-2",
            p { class: "text-[10px] text-gray-500",
                "Violet = sampled probed · Indigo = dispatch probed · Slate = shadow"
            }
            div { class: "overflow-x-auto pb-1",
                div {
                    class: "flex items-end gap-1 min-w-max h-32 px-1",
                    key: "timeline-{refresh_tick}",
                    for step in steps.iter() {
                        {
                            let pct = (step.duration_ms / max_ms).clamp(0.08, 1.0);
                            let bar_color = if step.is_shadow {
                                "bg-slate-400/90"
                            } else if step.sampled {
                                "bg-violet-500/85"
                            } else {
                                "bg-indigo-300/90"
                            };
                            let kind = if step.is_shadow {
                                "shadow"
                            } else if step.sampled {
                                "sampled"
                            } else {
                                "dispatch"
                            };
                            let title = format!(
                                "step {}: {:.1} ms ({})",
                                step.local_step,
                                step.duration_ms,
                                kind,
                            );
                            let height_pct = pct * 100.0;
                            rsx! {
                                div {
                                    key: "{step.local_step}",
                                    class: "flex flex-col items-center justify-end gap-1 min-w-[28px] h-full",
                                    title: "{title}",
                                    div {
                                        class: "w-5 rounded-t {bar_color} transition-all duration-500 ease-out",
                                        style: "height: {height_pct:.1}%",
                                    }
                                    span {
                                        class: "text-[9px] text-gray-400 tabular-nums leading-none",
                                        "{step.local_step}"
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
