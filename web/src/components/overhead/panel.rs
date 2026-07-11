//! TorchProbe overhead dashboard panel (presentation only).

use dioxus::prelude::*;
use probing_proto::prelude::{DataFrame, Ele};

use crate::api::{empty_dataframe, ApiClient};
use crate::components::common::{AppErrorDisplay, EmptyState};
use crate::components::stat_card::StatCard;
use crate::hooks::use_app_resource;
use crate::overhead::{
    dataframe_rows, df_scalar_f64, df_scalar_i64, format_overhead_pct, format_step_ms,
    parse_overhead_steps, table_missing_message, OverheadStep,
};
use crate::utils::error::AppError;

#[component]
pub fn TorchOverheadPanel(refresh_tick: u32) -> Element {
    let summary = use_app_resource(move || {
        let _ = refresh_tick;
        async move { ApiClient::new().fetch_overhead_summary().await }
    });
    let recent = use_app_resource(move || {
        let _ = refresh_tick;
        async move { ApiClient::new().fetch_overhead_recent_steps().await }
    });
    let train_step = use_app_resource(move || {
        let _ = refresh_tick;
        async move { ApiClient::new().fetch_overhead_train_step_median().await }
    });
    let nccl_skip = use_signal(|| false);
    let nccl = use_app_resource(move || {
        let _ = refresh_tick;
        let mut nccl_skip = nccl_skip;
        async move {
            if nccl_skip() {
                return Ok(empty_dataframe());
            }
            match ApiClient::new().fetch_overhead_nccl_counters().await {
                Ok(None) => {
                    nccl_skip.set(true);
                    Ok(empty_dataframe())
                }
                Ok(Some(df)) => Ok(df),
                Err(e) => Err(e),
            }
        }
    });

    let summary_res = summary.suspend()?();
    let recent_res = recent.suspend()?();
    let train_step_res = train_step.suspend()?();
    let nccl_res = nccl.suspend()?();
    overhead_body(&summary_res, &recent_res, &train_step_res, &nccl_res)
}

#[component]
fn OverheadAlerts(shadow_baseline: i64, shadow_n: i64) -> Element {
    if shadow_baseline == 0 {
        return rsx! {
            div {
                class: "rounded-md border border-slate-300 bg-slate-50 px-3 py-2 text-xs text-slate-800",
                "Shadow baseline is disabled ("
                code { class: "font-mono", "shadow=off" }
                "). Set "
                code { class: "font-mono", "probing.torch.profiling=on,shadow=4:1" }
                " to enable in-run overhead estimates."
            }
        };
    }
    if shadow_n > 0 && shadow_n < 5 {
        return rsx! {
            div { class: "rounded-md border border-amber-300 bg-amber-50 px-3 py-2 text-xs text-amber-900",
                "Only {shadow_n} shadow baseline step(s) in the window — overhead estimate may be unreliable. Wait for more training steps."
            }
        };
    }
    if shadow_baseline > 0 && shadow_n == 0 {
        return rsx! {
            div { class: "rounded-md border border-blue-200 bg-blue-50 px-3 py-2 text-xs text-blue-900",
                "Collecting shadow baseline steps — need at least one shadow step per cadence cycle before overhead % is meaningful."
            }
        };
    }
    rsx! { div {} }
}

fn overhead_body(
    summary: &Result<DataFrame, AppError>,
    recent: &Result<DataFrame, AppError>,
    train_step: &Result<DataFrame, AppError>,
    nccl: &Result<DataFrame, AppError>,
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

    let summary_df = summary.as_ref().ok();
    let recent_df = recent.as_ref().ok();
    let row_count = recent_df.map(dataframe_rows).unwrap_or(0);

    let shadow_baseline = summary_df
        .and_then(|df| df_scalar_i64(df, "shadow_baseline", 0))
        .unwrap_or(1);
    let shadow_n = summary_df
        .and_then(|df| df_scalar_i64(df, "shadow_n", 0))
        .unwrap_or(0);

    if row_count == 0 {
        return rsx! {
            EmptyState {
                message: "No python.torch_step_timing rows yet — enable SET probing.torch.profiling=on and wait for a few shadow cycles (default 4:1).".to_string()
            }
        };
    }

    if shadow_baseline == 0 {
        return rsx! {
            div { class: "space-y-3",
                OverheadAlerts { shadow_baseline, shadow_n }
                p { class: "text-xs text-gray-500",
                    "Timing rows exist but shadow baseline is off — enable shadow cadence to compare probed vs baseline steps."
                }
            }
        };
    }

    let probed_median_ms = summary_df.and_then(|df| df_scalar_f64(df, "probed_median_ms", 0));
    let sampled_median_ms = summary_df.and_then(|df| df_scalar_f64(df, "sampled_median_ms", 0));
    let shadow_median_ms = summary_df.and_then(|df| df_scalar_f64(df, "shadow_median_ms", 0));
    let probed_mean_ms = summary_df.and_then(|df| df_scalar_f64(df, "probed_mean_ms", 0));
    let shadow_mean_ms = summary_df.and_then(|df| df_scalar_f64(df, "shadow_mean_ms", 0));
    let probed_n = summary_df
        .and_then(|df| df_scalar_i64(df, "probed_n", 0))
        .unwrap_or(0);
    let sampled_n = summary_df
        .and_then(|df| df_scalar_i64(df, "sampled_n", 0))
        .unwrap_or(0);
    let latest_step = summary_df
        .and_then(|df| df_scalar_i64(df, "latest_step", 0))
        .unwrap_or(-1);
    let shadow_normal = summary_df
        .and_then(|df| df_scalar_i64(df, "shadow_normal", 0))
        .unwrap_or(4);
    let sample_rate = summary_df.and_then(|df| df_scalar_f64(df, "sample_rate", 0));
    let sample_mode = summary_df.and_then(|df| {
        df.names
            .iter()
            .position(|n| n == "sample_mode")
            .and_then(|ci| {
                summary_df?
                    .cols
                    .get(ci)
                    .filter(|col| !col.is_empty())
                    .map(|col| match &col.get(0) {
                        Ele::Text(s) => s.clone(),
                        other => format!("{other:?}"),
                    })
            })
    });
    let train_step_median_ms = train_step
        .as_ref()
        .ok()
        .and_then(|df| df_scalar_f64(df, "train_step_median_ms", 0));

    let (hook_tax_value, hook_tax_hint) = match (probed_median_ms, shadow_median_ms) {
        (Some(p), Some(s)) => format_overhead_pct(p, s, "Hook tax (all probed steps vs shadow)"),
        _ => (
            "—".to_string(),
            Some("Collecting shadow baseline steps…".to_string()),
        ),
    };
    let (sampled_overhead_value, sampled_overhead_hint) =
        match (sampled_median_ms, shadow_median_ms) {
            (Some(p), Some(s)) if sampled_n > 0 => {
                format_overhead_pct(p, s, "Active sample steps vs shadow")
            }
            _ => ("—".to_string(), None),
        };
    let (total_overhead_value, total_overhead_hint) = match (probed_mean_ms, shadow_mean_ms) {
        (Some(p), Some(s)) => {
            format_overhead_pct(p, s, "Amortized: mean(all probed) vs mean(shadow)")
        }
        _ => (
            "—".to_string(),
            Some("Collecting shadow baseline steps…".to_string()),
        ),
    };

    let cadence = format!("{shadow_normal}:{shadow_baseline}");
    let sampling_note = match (sample_mode.as_deref(), sample_rate) {
        (Some(mode), Some(rate)) => format!(" · {mode} @ {:.0}%", rate * 100.0),
        (Some(mode), None) => format!(" · {mode}"),
        _ => String::new(),
    };
    let steps = recent_df.map(parse_overhead_steps).unwrap_or_default();

    rsx! {
        div { class: "space-y-4",
            OverheadAlerts { shadow_baseline, shadow_n }
            p { class: "text-xs text-gray-500",
                "Shadow cadence {cadence}{sampling_note} · Total overhead = mean(all probed)/mean(shadow)−1 (amortized) · Hook tax = median(probed)/median(shadow)−1 (cheap dispatch path) · probed median is wall time between optimizer steps · train.step is compute-only"
            }
            div { class: "grid grid-cols-2 sm:grid-cols-3 lg:grid-cols-4 gap-3",
                StatCard {
                    label: "Total overhead".to_string(),
                    value: total_overhead_value,
                    hint: total_overhead_hint,
                }
                StatCard {
                    label: "Hook tax".to_string(),
                    value: hook_tax_value,
                    hint: hook_tax_hint,
                }
                StatCard {
                    label: "Sampled overhead".to_string(),
                    value: sampled_overhead_value,
                    hint: sampled_overhead_hint.or_else(|| {
                        if sampled_n > 0 {
                            Some(format!("{sampled_n} sampled steps in window"))
                        } else {
                            Some("No sampled steps in window yet".to_string())
                        }
                    }),
                }
                StatCard {
                    label: "Probed median".to_string(),
                    value: probed_median_ms.map(format_step_ms).unwrap_or_else(|| "—".to_string()),
                    hint: Some(format!("{probed_n} probed steps")),
                }
                StatCard {
                    label: "Sampled median".to_string(),
                    value: sampled_median_ms.map(format_step_ms).unwrap_or_else(|| "—".to_string()),
                    hint: if sampled_n > 0 {
                        Some(format!("{sampled_n} sampled steps"))
                    } else {
                        Some("No sampled steps in window yet".to_string())
                    },
                }
                StatCard {
                    label: "Shadow median".to_string(),
                    value: shadow_median_ms.map(format_step_ms).unwrap_or_else(|| "—".to_string()),
                    hint: Some(format!("{shadow_n} baseline steps")),
                }
                StatCard {
                    label: "Latest step".to_string(),
                    value: latest_step.to_string(),
                    hint: None,
                }
                StatCard {
                    label: "train.step median".to_string(),
                    value: train_step_median_ms
                        .map(format_step_ms)
                        .unwrap_or_else(|| "—".to_string()),
                    hint: Some("Cross-check vs span timeline".to_string()),
                }
            }
            NcclOverheadFootnote { nccl: nccl.clone() }
            if let Err(err) = recent {
                AppErrorDisplay { error: err.clone(), title: Some("Recent steps query failed".to_string()) }
            } else if !steps.is_empty() {
                TorchOverheadTimeline { steps: steps.clone() }
            }
        }
    }
}

#[component]
fn NcclOverheadFootnote(nccl: Result<DataFrame, AppError>) -> Element {
    match nccl {
        Err(_) => rsx! {
            p { class: "text-[10px] text-gray-500 border-t border-gray-100 pt-3",
                "NCCL profiler: no in-run shadow step yet — use "
                code { class: "font-mono text-gray-500", "examples/run_nccl_profiler_bench.sh" }
                " for offline AllReduce overhead comparison."
            }
        },
        Ok(df) if dataframe_rows(&df) == 0 => rsx! {
            p { class: "text-[10px] text-gray-500 border-t border-gray-100 pt-3",
                "NCCL profiler inactive — Torch overhead above is TorchProbe only. Offline NCCL bench: "
                code { class: "font-mono text-gray-500", "examples/run_nccl_profiler_bench.sh" }
            }
        },
        Ok(df) => {
            let coll_events = df_scalar_i64(&df, "coll_events", 0).unwrap_or(0);
            let rows_written = df_scalar_i64(&df, "rows_written", 0).unwrap_or(0);
            let pool_exhausted = df_scalar_i64(&df, "pool_exhausted", 0).unwrap_or(0);
            let write_errors = df_scalar_i64(&df, "write_errors", 0).unwrap_or(0);
            rsx! {
                p { class: "text-[10px] text-gray-500 border-t border-gray-100 pt-3",
                    "NCCL profiler active · coll_events {coll_events} · rows_written {rows_written}"
                    if pool_exhausted > 0 || write_errors > 0 {
                        span { class: "text-amber-700",
                            " · pool_exhausted {pool_exhausted} · write_errors {write_errors}"
                        }
                    }
                    " · NCCL has no in-run shadow baseline; offline bench: "
                    code { class: "font-mono", "examples/run_nccl_profiler_bench.sh" }
                }
            }
        }
    }
}

#[component]
fn TorchOverheadTimeline(steps: Vec<OverheadStep>) -> Element {
    let max_ms = steps
        .iter()
        .map(|s| s.duration_ms)
        .fold(0.0f64, f64::max)
        .max(1.0);

    rsx! {
        div { class: "space-y-2",
            p { class: "text-[10px] text-gray-500",
                "Recent step wall time · violet = probed+sampled · indigo = probed dispatch only · slate = shadow"
            }
            div { class: "overflow-x-auto pb-1",
                div { class: "flex items-end gap-1 min-w-max h-32 px-1",
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
                            rsx! {
                                div {
                                    class: "flex flex-col items-center justify-end gap-1 min-w-[28px] h-full",
                                    title: "{title}",
                                    div {
                                        class: "w-6 rounded-t-sm {bar_color}",
                                        style: "height: {pct * 100.0}%",
                                    }
                                    span { class: "text-[9px] font-mono text-gray-500", "{step.local_step}" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
