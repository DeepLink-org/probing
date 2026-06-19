use std::collections::HashSet;

use dioxus::prelude::*;

use crate::api::{ApiClient, VariableRecord};
use crate::components::colors::colors;
use crate::components::common::{query_result, AppErrorDisplay};
use crate::hooks::use_app_resource;
use crate::utils::error::AppError;

use super::shared::PREVIEW_RECORD_LIMIT;

#[component]
pub fn ActiveTracesPanel(
    poll: Signal<u32>,
    refresh_key: Signal<u32>,
    on_view_records: EventHandler<String>,
) -> Element {
    let traces = use_app_resource(move || {
        let _ = poll();
        let _ = refresh_key();
        async move { ApiClient::new().get_trace_info().await }
    });
    let mut stop_trace = use_action(move |func: String| async move {
        ApiClient::new().stop_trace(&func).await?;
        refresh_key.set(refresh_key() + 1);
        Ok::<(), AppError>(())
    });

    let trace_list = traces.suspend()?();

    query_result(
        trace_list,
        |list| list.is_empty(),
        "No active traces. Pick a function from the catalog and click Trace.",
        move |active| {
            rsx! {
                div { class: "divide-y divide-gray-100",
                    for func in active {
                        ActiveTraceRow {
                            key: "{func}",
                            function: func,
                            poll,
                            refresh_key,
                            on_view_records,
                            stop_pending: stop_trace.pending(),
                            on_stop: move |name| stop_trace.call(name),
                        }
                    }
                }
                if stop_trace.pending() {
                    div { class: "px-4 py-2 text-xs text-gray-500 border-t border-gray-100",
                        "Stopping trace…"
                    }
                } else if let Some(Err(err)) = stop_trace.value() {
                    div { class: "px-4 py-2 border-t border-gray-100",
                        AppErrorDisplay {
                            error: AppError::Api(err.to_string()),
                            title: Some("Stop failed".to_string()),
                        }
                    }
                }
            }
        },
    )
}

#[component]
fn ActiveTraceRow(
    function: String,
    poll: Signal<u32>,
    refresh_key: Signal<u32>,
    on_view_records: EventHandler<String>,
    stop_pending: bool,
    on_stop: EventHandler<String>,
) -> Element {
    let records = use_app_resource({
        let function = function.clone();
        move || {
            let func = function.clone();
            let _ = poll();
            let _ = refresh_key();
            async move { fetch_trace_preview(&func).await }
        }
    });
    let snapshot = records.read();
    let refreshing = records.pending();
    let latest = snapshot
        .as_ref()
        .and_then(|r| r.as_ref().ok())
        .map(|rows| latest_from_records(rows))
        .unwrap_or_default();

    rsx! {
        div { class: "px-4 py-3 hover:bg-gray-50/80",
            div { class: "flex items-start gap-3",
                div { class: "min-w-0 flex-1",
                    span { class: "inline-flex items-center gap-1.5 text-xs font-medium text-emerald-800 bg-emerald-50 border border-emerald-200 px-2 py-0.5 rounded-full mb-1",
                        span { class: "w-1.5 h-1.5 rounded-full bg-emerald-500" }
                        "tracing"
                    }
                    p { class: "font-mono text-sm text-gray-900 break-all", "{function}" }
                    if let Some(result) = snapshot.as_ref() {
                        match result {
                            Ok(_) if latest.is_empty() => rsx! {
                                p { class: "mt-1.5 text-xs text-gray-400", "Waiting for variable updates…" }
                            },
                            Ok(_) => rsx! {
                                div { class: "mt-2 space-y-1.5",
                                    div { class: "flex flex-wrap gap-1.5",
                                        for (name, value, ty) in latest {
                                            VariablePreviewChip { name, value, ty }
                                        }
                                    }
                                    if refreshing {
                                        p { class: "text-[11px] text-gray-400", "Updating…" }
                                    }
                                }
                            },
                            Err(_) => rsx! {
                                p { class: "mt-1.5 text-xs text-amber-700", "Preview unavailable" }
                            },
                        }
                    } else {
                        p { class: "mt-1.5 text-xs text-gray-400", "Loading latest values…" }
                    }
                }
                div { class: "flex shrink-0 gap-2 pt-0.5",
                    button {
                        class: "px-2.5 py-1.5 text-xs rounded-md border border-gray-300 bg-white hover:bg-gray-50",
                        onclick: {
                            let func = function.clone();
                            move |_| on_view_records.call(func.clone())
                        },
                        "Records"
                    }
                    button {
                        class: format!(
                            "px-2.5 py-1.5 text-xs rounded-md text-white bg-{} hover:bg-{} disabled:opacity-50",
                            colors::ERROR,
                            colors::ERROR_HOVER,
                        ),
                        disabled: stop_pending,
                        onclick: {
                            let func = function.clone();
                            move |_| on_stop.call(func.clone())
                        },
                        "Stop"
                    }
                }
            }
        }
    }
}

#[component]
fn VariablePreviewChip(name: String, value: String, ty: String) -> Element {
    rsx! {
        span {
            class: format!(
                "inline-flex items-center gap-1 max-w-full text-xs font-mono px-2 py-1 rounded border bg-{} border-{}",
                colors::CONTENT_ACCENT_BG,
                colors::CONTENT_ACCENT_BORDER,
            ),
            title: "{name} = {value} ({ty})",
            span { class: "text-gray-600 shrink-0", "{name}" }
            span { class: "text-gray-400 shrink-0", "=" }
            span { class: "text-gray-900 truncate max-w-[10rem]", "{value}" }
            span { class: "text-[10px] text-gray-400 shrink-0", "{ty}" }
        }
    }
}

async fn fetch_trace_preview(function: &str) -> Result<Vec<VariableRecord>, AppError> {
    let client = ApiClient::new();
    let rows = client
        .get_trace_variables(Some(function), Some(PREVIEW_RECORD_LIMIT))
        .await?;
    if !rows.is_empty() {
        return Ok(rows);
    }
    let all = client
        .get_trace_variables(None, Some(PREVIEW_RECORD_LIMIT))
        .await?;
    Ok(filter_records_for_function(all, function))
}

fn filter_records_for_function(records: Vec<VariableRecord>, function: &str) -> Vec<VariableRecord> {
    let short = function.rsplit('.').next().unwrap_or(function);
    records
        .into_iter()
        .filter(|r| {
            r.function_name == function
                || r.function_name == short
                || r.function_name.ends_with(&format!(".{short}"))
        })
        .collect()
}

fn latest_from_records(records: &[VariableRecord]) -> Vec<(String, String, String)> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for r in records {
        if seen.insert(r.variable_name.clone()) {
            out.push((
                r.variable_name.clone(),
                r.value.clone(),
                r.value_type.clone(),
            ));
        }
    }
    out
}
