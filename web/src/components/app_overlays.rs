//! Viewport overlays mounted at app root (monitors, source viewer, …).

use dioxus::prelude::*;

use crate::api::OVERHEAD_POLL_MS;
use crate::components::common::AsyncBoundary;
use crate::components::icon::Icon;
use crate::components::overhead::TorchOverheadPanel;
use crate::components::overlay_shell::{OverlayAccent, OverlayShell};
use crate::components::source_viewer::SourceViewerOverlay;
use crate::hooks::{use_page_visible, use_poll_tick_gated};
use crate::state::overlays::{app_overlay, close_app_overlay, AppOverlay, SidebarMonitor};
use crate::state::scroll_lock::unlock_body_scroll;
use crate::state::ui_tasks::{
    cancel_all_running_ui_tasks, cancel_ui_task, clear_finished_ui_tasks, ui_tasks_snapshot,
    UiTask, UiTaskStatus, UI_TASK_TICK,
};

/// Renders whichever global overlay is active.
#[component]
pub fn AppOverlays() -> Element {
    let open = app_overlay();

    use_effect(move || {
        if app_overlay().is_none() {
            unlock_body_scroll();
        }
    });

    match open {
        None => rsx! {},
        Some(AppOverlay::SourceViewer(_)) => rsx! {
            SourceViewerOverlay {}
        },
        Some(AppOverlay::Monitor(SidebarMonitor::Tasks)) => rsx! {
            TasksMonitorOverlay {
                on_close: move |_| close_app_overlay(),
            }
        },
        Some(AppOverlay::Monitor(SidebarMonitor::Overhead)) => rsx! {
            OverheadMonitorOverlay {
                on_close: move |_| close_app_overlay(),
            }
        },
    }
}

#[component]
fn TasksMonitorOverlay(on_close: EventHandler<()>) -> Element {
    let _tick = UI_TASK_TICK.read();
    let tasks = ui_tasks_snapshot();
    let now_ms = js_sys::Date::now() as u64;
    let running = tasks.iter().filter(|t| t.is_running()).count();
    let has_finished = tasks.iter().any(|t| !t.is_running());

    let subtitle = if running > 0 {
        format!("{running} active · {} total · Esc to close", tasks.len())
    } else {
        format!("{} tasks · Esc to close", tasks.len())
    };

    let header_actions = rsx! {
        div { class: "flex items-center gap-2 shrink-0 mr-1",
            if running > 0 {
                button {
                    r#type: "button",
                    class: "text-xs text-gray-500 hover:text-red-600 transition-colors px-2 py-1 rounded hover:bg-red-50",
                    title: "Cancel all running tasks",
                    onclick: move |e| {
                        e.stop_propagation();
                        cancel_all_running_ui_tasks();
                    },
                    "Cancel all"
                }
            }
            if has_finished {
                button {
                    r#type: "button",
                    class: "text-xs text-gray-500 hover:text-gray-800 transition-colors px-2 py-1 rounded hover:bg-gray-100",
                    onclick: move |e| {
                        e.stop_propagation();
                        clear_finished_ui_tasks();
                    },
                    "Clear finished"
                }
            }
        }
    };

    rsx! {
        OverlayShell {
            title: "Background tasks".to_string(),
            subtitle: subtitle,
            accent: OverlayAccent::Blue,
            close_label: "Close task monitor".to_string(),
            on_close: on_close,
            header_icon: rsx! {
                Icon { icon: &icondata::AiUnorderedListOutlined, class: "w-5 h-5" }
            },
            header_actions: header_actions,
            if tasks.is_empty() {
                p { class: "text-sm text-gray-500 text-center py-12",
                    "No background tasks"
                }
            } else {
                div { class: "space-y-2",
                    for task in tasks.iter().rev() {
                        TaskRow { key: "{task.id}", task: task.clone(), now_ms: now_ms }
                    }
                }
            }
        }
    }
}

#[component]
fn OverheadMonitorOverlay(on_close: EventHandler<()>) -> Element {
    let visible = use_page_visible();
    let poll = use_poll_tick_gated(OVERHEAD_POLL_MS, Some(visible));
    let refresh_tick = poll();

    rsx! {
        OverlayShell {
            title: "Torch profiling overhead".to_string(),
            subtitle: format!(
                "TorchProbe in-run cost vs shadow baseline · refreshes every {}s · Esc to close",
                OVERHEAD_POLL_MS / 1000
            ),
            accent: OverlayAccent::Emerald,
            close_label: "Close overhead monitor".to_string(),
            on_close: on_close,
            header_icon: rsx! {
                Icon { icon: &icondata::CgPerformance, class: "w-5 h-5" }
            },
            header_actions: rsx! { div {} },
            AsyncBoundary {
                message: Some("Loading overhead…".to_string()),
                TorchOverheadPanel { refresh_tick: refresh_tick }
            }
        }
    }
}

#[component]
fn TaskRow(task: UiTask, now_ms: u64) -> Element {
    let row_class = match task.status {
        UiTaskStatus::Running => "bg-blue-50/80 border-blue-200",
        UiTaskStatus::Done => "bg-white border-gray-200 opacity-80",
        UiTaskStatus::Failed => "bg-red-50 border-red-200",
        UiTaskStatus::Cancelled => "bg-gray-50 border-gray-200 opacity-70",
    };

    let kind_label = task.kind.label();
    let elapsed = task.elapsed_label(now_ms);
    let task_id = task.id;

    rsx! {
        div {
            class: "flex items-start gap-2 px-3 py-2.5 rounded-lg border text-sm leading-snug {row_class}",
            title: task.error.clone().unwrap_or_else(|| task.label.clone()),
            match task.status {
                UiTaskStatus::Running => rsx! {
                    span {
                        class: "inline-block w-3 h-3 border-2 border-blue-500 border-t-transparent rounded-full animate-spin shrink-0 mt-0.5"
                    }
                },
                UiTaskStatus::Done => rsx! {
                    Icon { icon: &icondata::AiCheckOutlined, class: "w-4 h-4 text-emerald-600 shrink-0 mt-0.5" }
                },
                UiTaskStatus::Failed => rsx! {
                    Icon { icon: &icondata::AiCloseCircleOutlined, class: "w-4 h-4 text-red-500 shrink-0 mt-0.5" }
                },
                UiTaskStatus::Cancelled => rsx! {
                    Icon { icon: &icondata::AiStopOutlined, class: "w-4 h-4 text-gray-400 shrink-0 mt-0.5" }
                },
            }
            div { class: "flex-1 min-w-0",
                div { class: "text-gray-900 truncate font-medium", "{task.label}" }
                div { class: "text-gray-500 truncate text-xs mt-0.5",
                    span { "{kind_label}" }
                    if let Some(detail) = &task.detail {
                        span { " · {detail}" }
                    }
                }
                if let Some(err) = &task.error {
                    div { class: "text-red-600 truncate mt-1 text-xs", "{err}" }
                }
                if task.status == UiTaskStatus::Cancelled {
                    div { class: "text-gray-500 truncate mt-1 text-xs", "Cancelled" }
                }
            }
            div { class: "flex flex-col items-end gap-1 shrink-0",
                span { class: "text-gray-500 tabular-nums text-xs", "{elapsed}" }
                if task.is_running() {
                    button {
                        r#type: "button",
                        class: "p-1 rounded text-gray-400 hover:text-red-600 hover:bg-red-50 transition-colors",
                        title: if task.group_id.is_some() { "Cancel session" } else { "Cancel task" },
                        onclick: move |e| {
                            e.stop_propagation();
                            cancel_ui_task(task_id);
                        },
                        Icon { icon: &icondata::AiCloseOutlined, class: "w-3.5 h-3.5" }
                    }
                }
            }
        }
    }
}
