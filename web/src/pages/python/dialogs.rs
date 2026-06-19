use dioxus::html::events::KeyboardEvent;
use dioxus::html::input_data::keyboard_types::Key;
use dioxus::prelude::*;

use crate::api::ApiClient;
use crate::components::colors::colors;
use crate::components::common::{AppErrorDisplay, LoadingState};
use crate::components::dataframe_view::DataFrameView;
use crate::components::icon::Icon;
use crate::hooks::use_app_resource;
use crate::utils::error::AppError;

use super::shared::{StartTraceDraft, POLL_MS};

#[component]
pub fn StartTraceDialog(
    draft: Signal<StartTraceDraft>,
    on_close: EventHandler<()>,
    on_started: EventHandler<()>,
) -> Element {
    #[allow(clippy::redundant_closure)]
    let mut local = use_signal(|| draft());
    let mut start_trace = use_action(
        move |(function, watch, print_to_terminal): (String, String, bool)| async move {
            let watch_list: Vec<String> = if watch.trim().is_empty() {
                vec![]
            } else {
                watch
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            };
            let resp = ApiClient::new()
                .start_trace(&function, Some(watch_list), print_to_terminal)
                .await?;
            if resp.success {
                on_started.call(());
                Ok(())
            } else {
                Err(AppError::Api(
                    resp.error
                        .or(resp.message)
                        .unwrap_or_else(|| "Start trace failed".to_string()),
                ))
            }
        },
    );

    rsx! {
        div {
            class: "fixed inset-0 z-50 flex items-end sm:items-center justify-center p-0 sm:p-4",
            tabindex: "-1",
            onkeydown: move |e: KeyboardEvent| {
                if e.key() == Key::Escape && !start_trace.pending() {
                    on_close.call(());
                }
            },
            div {
                class: "absolute inset-0 bg-slate-900/40 backdrop-blur-sm",
                onclick: move |_| {
                    if !start_trace.pending() {
                        on_close.call(());
                    }
                },
            }
            div {
                class: "relative w-full sm:max-w-md bg-white sm:rounded-xl shadow-2xl border border-gray-200 p-5",
                onclick: move |e| e.stop_propagation(),
                div { class: "flex items-center gap-2 mb-4",
                    Icon { icon: &icondata::SiPython, class: "w-5 h-5 text-blue-600" }
                    h3 { class: "text-base font-semibold text-gray-900", "Start tracing" }
                }
                div { class: "space-y-4",
                    div {
                        label { class: "block text-xs font-medium text-gray-600 mb-1", "Function" }
                        input {
                            class: "w-full px-3 py-2 text-sm font-mono border border-gray-200 rounded-md bg-gray-50",
                            readonly: true,
                            value: "{local().function}",
                        }
                    }
                    div {
                        label { class: "block text-xs font-medium text-gray-600 mb-1",
                            "Watch variables (comma-separated)"
                        }
                        input {
                            class: "w-full px-3 py-2 text-sm font-mono border border-gray-300 rounded-md focus:outline-none focus:ring-2 focus:ring-blue-500/30",
                            placeholder: "x, y, loss",
                            value: "{local().watch}",
                            oninput: move |ev| {
                                local.write().watch = ev.value();
                            },
                        }
                    }
                    label { class: "flex items-center gap-2 text-sm text-gray-700 cursor-pointer",
                        input {
                            r#type: "checkbox",
                            class: "rounded border-gray-300",
                            checked: local().print_to_terminal,
                            onchange: move |ev| {
                                local.write().print_to_terminal = ev.checked();
                            },
                        }
                        "Print changes to terminal (otherwise DB only)"
                    }
                    if start_trace.pending() {
                        LoadingState { message: Some("Starting trace…".to_string()) }
                    } else if let Some(Err(err)) = start_trace.value() {
                        AppErrorDisplay {
                            error: AppError::Api(err.to_string()),
                            title: Some("Start failed".to_string()),
                        }
                    }
                    div { class: "flex justify-end gap-2 pt-2",
                        button {
                            class: "px-3 py-2 text-sm rounded-md border border-gray-300 hover:bg-gray-50 disabled:opacity-50",
                            disabled: start_trace.pending(),
                            onclick: move |_| on_close.call(()),
                            "Cancel"
                        }
                        button {
                            class: format!(
                                "px-4 py-2 text-sm rounded-md text-white bg-{} hover:bg-{} disabled:opacity-50",
                                colors::PRIMARY,
                                colors::PRIMARY_HOVER,
                            ),
                            disabled: start_trace.pending(),
                            onclick: move |_| {
                                let d = local();
                                start_trace.call((d.function, d.watch, d.print_to_terminal));
                            },
                            if start_trace.pending() { "Starting…" } else { "Start" }
                        }
                    }
                }
            }
        }
    }
}

#[component]
pub fn RecordsModal(function: String, poll: Signal<u32>, on_close: EventHandler<()>) -> Element {
    let function_label = function.clone();
    let records = use_app_resource({
        let function = function.clone();
        move || {
            let func = function.clone();
            let _ = poll();
            async move {
                ApiClient::new()
                    .get_variable_records(Some(&func), Some(100))
                    .await
            }
        }
    });
    let refreshing = records.pending();
    let snapshot = records.read();

    rsx! {
        div {
            class: "fixed inset-0 z-50 flex items-end sm:items-center justify-center p-0 sm:p-4",
            tabindex: "-1",
            onkeydown: move |e: KeyboardEvent| {
                if e.key() == Key::Escape {
                    on_close.call(());
                }
            },
            div {
                class: "absolute inset-0 bg-slate-900/40 backdrop-blur-sm",
                onclick: move |_| on_close.call(()),
            }
            div {
                class: "relative w-full sm:max-w-5xl max-h-[90vh] flex flex-col bg-white sm:rounded-xl shadow-2xl border border-gray-200 overflow-hidden",
                div { class: "flex items-center justify-between gap-3 px-4 py-3 border-b border-gray-200 bg-gray-50/80",
                    div { class: "min-w-0",
                        h3 { class: "text-base font-semibold text-gray-900 truncate", "Variable records" }
                        p { class: "text-xs font-mono text-gray-500 truncate", "{function_label}" }
                    }
                    div { class: "flex items-center gap-2 shrink-0",
                        if refreshing && snapshot.as_ref().is_some() {
                            span { class: "text-[11px] text-gray-500", "Updating…" }
                        }
                        span { class: "text-[11px] text-gray-500 tabular-nums",
                            "Auto {POLL_MS / 1000}s"
                        }
                        button {
                            class: format!(
                                "px-3 py-1.5 text-sm rounded-md bg-{} hover:bg-{}",
                                colors::BTN_SECONDARY_BG,
                                colors::BTN_SECONDARY_HOVER,
                            ),
                            onclick: move |_| on_close.call(()),
                            "Close"
                        }
                    }
                }
                div { class: "flex-1 overflow-auto p-4",
                    if let Some(result) = snapshot.as_ref() {
                        match result {
                            Ok(df) => rsx! {
                                div { class: "rounded-lg border border-gray-200 overflow-hidden",
                                    DataFrameView { df: df.clone(), on_row_click: None }
                                }
                            },
                            Err(err) => rsx! {
                                AppErrorDisplay {
                                    error: err.clone(),
                                    title: Some("Load failed".to_string()),
                                }
                            },
                        }
                    } else {
                        LoadingState { message: Some("Loading records…".to_string()) }
                    }
                }
            }
        }
    }
}
