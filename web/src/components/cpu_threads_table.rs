//! Top CPU threads — name-first table with stack drill-down.

use dioxus::prelude::*;
use dioxus_router::use_navigator;

use crate::api::{format_cpu_ms, CpuThreadRow};
use crate::app::Route;
use crate::components::colors::colors;
use crate::state::investigation::{set_thread_context, INVESTIGATION_CONTEXT};

#[component]
pub fn CpuThreadsTable(threads: Vec<CpuThreadRow>) -> Element {
    let navigator = use_navigator();
    let pid = INVESTIGATION_CONTEXT.read().pid;

    rsx! {
        div { class: "overflow-x-auto",
            table { class: "w-full border-collapse text-sm",
                thead {
                    tr { class: "border-b border-gray-200 text-left text-xs uppercase tracking-wide text-gray-500",
                        th { class: "py-2 pr-4 font-medium", "Thread" }
                        th { class: "py-2 pr-4 font-medium", "State" }
                        th { class: "py-2 pr-4 font-medium text-right", "User" }
                        th { class: "py-2 pr-4 font-medium text-right", "Kernel" }
                        th { class: "py-2 pr-4 font-medium", "Waiting on" }
                        th { class: "py-2 font-medium text-right", "Actions" }
                    }
                }
                tbody {
                    for row in threads {
                        {
                            let tid = row.tid;
                            let tid_for_stack = row.tid.to_string();
                            let name = row.name.clone();
                            let has_named = !row.name.starts_with("thread-");
                            rsx! {
                                tr { class: "border-b border-gray-100 last:border-0 hover:bg-gray-50",
                                    td { class: "py-3 pr-4 align-top",
                                        div { class: "font-medium text-gray-900", "{row.name}" }
                                        if has_named {
                                            p { class: "text-xs text-gray-400 font-mono mt-0.5", "tid {tid}" }
                                        }
                                    }
                                    td { class: "py-3 pr-4 align-top",
                                        span {
                                            class: "inline-flex px-2 py-0.5 rounded text-xs font-mono bg-gray-100 text-gray-700",
                                            "{row.state}"
                                        }
                                    }
                                    td { class: "py-3 pr-4 align-top text-right font-mono text-blue-700",
                                        "{format_cpu_ms(row.delta_user_ns)}"
                                    }
                                    td { class: "py-3 pr-4 align-top text-right font-mono text-amber-700",
                                        "{format_cpu_ms(row.delta_sys_ns)}"
                                    }
                                    td { class: "py-3 pr-4 align-top text-gray-500 font-mono text-xs",
                                        {row.wchan.clone().unwrap_or_else(|| "—".to_string())}
                                    }
                                    td { class: "py-3 align-top text-right",
                                        div { class: "inline-flex flex-wrap justify-end gap-2",
                                            button {
                                                class: format!(
                                                    "text-xs font-medium text-{} hover:underline whitespace-nowrap",
                                                    colors::PRIMARY
                                                ),
                                                onclick: {
                                                    let name = name.clone();
                                                    move |_| {
                                                        set_thread_context(tid, Some(&name), pid);
                                                        navigator.push(Route::StackWithTidPage {
                                                            tid: tid_for_stack.clone(),
                                                        });
                                                    }
                                                },
                                                "Stack"
                                            }
                                            button {
                                                class: "text-xs font-medium text-gray-600 hover:underline whitespace-nowrap",
                                                onclick: {
                                                    let name = name.clone();
                                                    move |_| {
                                                        set_thread_context(tid, Some(&name), pid);
                                                        navigator.push(Route::TracesPage {});
                                                    }
                                                },
                                                "Spans"
                                            }
                                            button {
                                                class: format!(
                                                    "text-xs font-medium text-{} hover:underline whitespace-nowrap",
                                                    colors::CONTENT_ACCENT_TEXT
                                                ),
                                                onclick: {
                                                    let name = name.clone();
                                                    move |_| {
                                                        set_thread_context(tid, Some(&name), pid);
                                                        navigator.push(Route::ProfilingViewPage {
                                                            view: "pprof".to_string(),
                                                        });
                                                    }
                                                },
                                                "Profile"
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
