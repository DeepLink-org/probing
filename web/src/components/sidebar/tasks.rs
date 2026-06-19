//! Sidebar task queue — compact summary row + click-to-open popover.

use dioxus::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::Element as DomElement;

use crate::components::icon::Icon;
use crate::components::sidebar::nav_item::SidebarSectionLabel;
use crate::state::sidebar::SIDEBAR_WIDTH;
use crate::state::ui_tasks::{
    cancel_all_running_ui_tasks, cancel_ui_task, clear_finished_ui_tasks, ui_tasks_snapshot,
    UiTask, UiTaskStatus, UI_TASK_TICK,
};

const TASK_TRIGGER_ID: &str = "sidebar-task-trigger";
const PANEL_GAP_PX: f64 = 6.0;

/// Anchor the popover above the trigger using viewport coordinates (avoids sidebar overflow clipping).
fn task_panel_style(trigger_id: &str) -> String {
    let Some(window) = web_sys::window() else {
        return String::new();
    };
    let Some(document) = window.document() else {
        return String::new();
    };
    let Some(el) = document.get_element_by_id(trigger_id) else {
        return String::new();
    };
    let Ok(html_el) = el.dyn_into::<DomElement>() else {
        return String::new();
    };
    let rect = html_el.get_bounding_client_rect();
    let top = rect.top();
    let left = rect.left();
    let width = rect.width();
    let vh = document
        .document_element()
        .map(|el| el.client_height() as f64)
        .unwrap_or(top + 200.0);
    // `bottom` = distance from viewport bottom to the panel's bottom edge (just above the button).
    let bottom = (vh - top + PANEL_GAP_PX).max(PANEL_GAP_PX);
    let max_h = (top - PANEL_GAP_PX - 8.0).max(120.0);
    format!(
        "position:fixed;bottom:{bottom}px;left:{left}px;width:{width}px;max-height:{max_h}px;z-index:50;",
    )
}

#[component]
pub fn SidebarTaskQueue() -> Element {
    let mut show_panel = use_signal(|| false);
    let mut panel_style = use_signal(String::new);
    let _tick = UI_TASK_TICK.read();
    let sidebar_width = *SIDEBAR_WIDTH.read();
    let tasks = ui_tasks_snapshot();
    let now_ms = js_sys::Date::now() as u64;
    let running = tasks.iter().filter(|t| t.is_running()).count();
    let has_finished = tasks.iter().any(|t| !t.is_running());
    let (summary_label, summary_running) = task_summary_line(&tasks);

    use_effect(move || {
        let _ = sidebar_width;
        if show_panel() {
            panel_style.set(task_panel_style(TASK_TRIGGER_ID));
        }
    });

    rsx! {
        div {
            class: "shrink-0 border-t border-slate-700/30 px-2 py-1.5 relative",
            SidebarSectionLabel { label: "Tasks" }
            button {
                id: TASK_TRIGGER_ID,
                class: "mt-1 w-full flex items-center gap-1.5 px-2 py-1.5 rounded-md border border-slate-700/50 \
                         bg-slate-800/30 hover:bg-slate-800/60 hover:border-slate-600/70 transition-colors \
                         text-left min-w-0",
                title: "Show task list",
                aria_expanded: if show_panel() { "true" } else { "false" },
                onclick: move |e| {
                    e.stop_propagation();
                    let opening = !show_panel();
                    show_panel.set(opening);
                    if opening {
                        panel_style.set(task_panel_style(TASK_TRIGGER_ID));
                    }
                },
                if summary_running {
                    span {
                        class: "inline-block w-2 h-2 border border-blue-400 border-t-transparent rounded-full animate-spin shrink-0"
                    }
                } else if tasks.is_empty() {
                    Icon { icon: &icondata::AiUnorderedListOutlined, class: "w-3 h-3 text-slate-500 shrink-0" }
                } else {
                    Icon { icon: &icondata::AiCheckOutlined, class: "w-3 h-3 text-slate-500 shrink-0" }
                }
                span {
                    class: "flex-1 min-w-0 text-[10px] text-slate-200 truncate font-medium",
                    "{summary_label}"
                }
                if running > 0 {
                    span {
                        class: "shrink-0 text-[10px] tabular-nums text-blue-300 font-medium",
                        "{running}"
                    }
                }
                Icon {
                    icon: if show_panel() { &icondata::AiUpOutlined } else { &icondata::AiDownOutlined },
                    class: "w-3 h-3 text-slate-500 shrink-0"
                }
            }

            if show_panel() {
                div {
                    class: "fixed inset-0 z-40",
                    onclick: move |_| show_panel.set(false),
                }
                div {
                    class: "flex flex-col rounded-lg border border-slate-600/80 bg-slate-900 shadow-2xl overflow-hidden",
                    style: "{panel_style()}",
                    role: "dialog",
                    aria_label: "Background tasks",
                    onclick: move |e| e.stop_propagation(),
                    div {
                        class: "flex items-center justify-between gap-2 px-2.5 py-2 border-b border-slate-700/60 bg-slate-800/80",
                        span { class: "text-[10px] font-medium text-slate-300",
                            if running > 0 {
                                "{running} active · {tasks.len()} total"
                            } else {
                                "{tasks.len()} tasks"
                            }
                        }
                        div { class: "flex items-center gap-2 shrink-0",
                            if running > 0 {
                                button {
                                    class: "text-[10px] text-slate-500 hover:text-red-300 transition-colors",
                                    title: "Cancel all running tasks",
                                    onclick: move |_| cancel_all_running_ui_tasks(),
                                    "Cancel all"
                                }
                            }
                            if has_finished {
                                button {
                                    class: "text-[10px] text-slate-500 hover:text-slate-300 transition-colors",
                                    onclick: move |_| clear_finished_ui_tasks(),
                                    "Clear"
                                }
                            }
                        }
                    }
                    if tasks.is_empty() {
                        p { class: "px-3 py-4 text-[10px] text-slate-500 text-center",
                            "No background tasks"
                        }
                    } else {
                        div { class: "overflow-y-auto min-h-0 p-1 space-y-0.5",
                            for task in tasks.iter().rev() {
                                TaskRow { key: "{task.id}", task: task.clone(), now_ms: now_ms }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn task_summary_line(tasks: &[UiTask]) -> (String, bool) {
    if let Some(task) = tasks.iter().find(|t| t.is_running()) {
        return (task.label.clone(), true);
    }
    if let Some(task) = tasks.last() {
        let suffix = match task.status {
            UiTaskStatus::Failed => " (failed)",
            UiTaskStatus::Cancelled => " (cancelled)",
            _ => "",
        };
        return (format!("{}{suffix}", task.label), false);
    }
    ("No background tasks".to_string(), false)
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
