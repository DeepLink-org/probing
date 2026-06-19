//! Sidebar task queue — running and recent background work.

use dioxus::prelude::*;

use crate::components::icon::Icon;
use crate::components::sidebar::nav_item::SidebarSectionLabel;
use crate::state::ui_tasks::{
    cancel_all_running_ui_tasks, cancel_ui_task, clear_finished_ui_tasks, ui_tasks_snapshot,
    UiTask, UiTaskStatus, UI_TASK_TICK,
};

#[component]
pub fn SidebarTaskQueue() -> Element {
    let _tick = UI_TASK_TICK.read();
    let tasks = ui_tasks_snapshot();
    let now_ms = js_sys::Date::now() as u64;
    let running = tasks.iter().filter(|t| t.is_running()).count();

    rsx! {
        div {
            class: "shrink-0 border-t border-slate-700/30 px-2 py-2 max-h-48 flex flex-col min-h-0",
            div { class: "flex items-center justify-between gap-2 mb-1",
                SidebarSectionLabel { label: "Tasks" }
                div { class: "flex items-center gap-2",
                    if running > 0 {
                        span {
                            class: "text-[10px] font-medium text-blue-300 tabular-nums",
                            "{running} active"
                        }
                        button {
                            class: "text-[10px] text-slate-500 hover:text-red-300 transition-colors",
                            title: "Cancel all running tasks",
                            onclick: move |_| cancel_all_running_ui_tasks(),
                            "Cancel all"
                        }
                    }
                }
            }
            if tasks.is_empty() {
                p { class: "px-2 py-1 text-[10px] text-slate-500",
                    "No background tasks"
                }
            } else {
                div { class: "space-y-0.5 overflow-y-auto min-h-0 flex-1",
                    for task in tasks.iter().rev() {
                        TaskRow { key: "{task.id}", task: task.clone(), now_ms: now_ms }
                    }
                }
                if tasks.iter().any(|t| !t.is_running()) {
                    button {
                        class: "mt-1.5 w-full px-2 py-0.5 text-[10px] rounded border border-slate-600 text-slate-500 hover:text-slate-300 hover:border-slate-500 transition-colors",
                        onclick: move |_| clear_finished_ui_tasks(),
                        "Clear finished"
                    }
                }
            }
        }
    }
}

#[component]
fn TaskRow(task: UiTask, now_ms: u64) -> Element {
    let row_class = match task.status {
        UiTaskStatus::Running => "bg-slate-800/40 border-slate-700/50",
        UiTaskStatus::Done => "border-transparent opacity-70",
        UiTaskStatus::Failed => "bg-red-950/30 border-red-900/40",
        UiTaskStatus::Cancelled => "border-transparent opacity-60",
    };

    let kind_label = task.kind.label();
    let elapsed = task.elapsed_label(now_ms);
    let task_id = task.id;

    rsx! {
        div {
            class: "flex items-start gap-1.5 px-2 py-1 rounded border text-[10px] leading-snug {row_class}",
            title: task.error.clone().unwrap_or_else(|| task.label.clone()),
            match task.status {
                UiTaskStatus::Running => rsx! {
                    span {
                        class: "inline-block w-2.5 h-2.5 border border-blue-400 border-t-transparent rounded-full animate-spin shrink-0 mt-0.5"
                    }
                },
                UiTaskStatus::Done => rsx! {
                    Icon { icon: &icondata::AiCheckOutlined, class: "w-3 h-3 text-emerald-400 shrink-0 mt-0.5" }
                },
                UiTaskStatus::Failed => rsx! {
                    Icon { icon: &icondata::AiCloseCircleOutlined, class: "w-3 h-3 text-red-400 shrink-0 mt-0.5" }
                },
                UiTaskStatus::Cancelled => rsx! {
                    Icon { icon: &icondata::AiStopOutlined, class: "w-3 h-3 text-slate-500 shrink-0 mt-0.5" }
                },
            }
            div { class: "flex-1 min-w-0",
                div { class: "text-slate-200 truncate font-medium", "{task.label}" }
                div { class: "text-slate-500 truncate",
                    span { "{kind_label}" }
                    if let Some(detail) = &task.detail {
                        span { " · {detail}" }
                    }
                }
                if let Some(err) = &task.error {
                    div { class: "text-red-300/90 truncate mt-0.5", "{err}" }
                }
                if task.status == UiTaskStatus::Cancelled {
                    div { class: "text-slate-500 truncate mt-0.5", "Cancelled" }
                }
            }
            div { class: "flex flex-col items-end gap-0.5 shrink-0",
                span { class: "text-slate-500 tabular-nums pt-0.5", "{elapsed}" }
                if task.is_running() {
                    button {
                        class: "p-0.5 rounded text-slate-500 hover:text-red-300 hover:bg-slate-700/50 transition-colors",
                        title: if task.group_id.is_some() { "Cancel session" } else { "Cancel task" },
                        onclick: move |e| {
                            e.stop_propagation();
                            cancel_ui_task(task_id);
                        },
                        Icon { icon: &icondata::AiCloseOutlined, class: "w-3 h-3" }
                    }
                }
            }
        }
    }
}
