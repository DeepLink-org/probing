//! Sidebar monitors — compact task + overhead summary rows; overlays render at app root.

use dioxus::prelude::*;

use crate::api::ApiClient;
use crate::components::icon::Icon;
use crate::components::overhead::{
    overhead_trigger_label, table_missing_trigger_label, OVERHEAD_POLL_MS,
};
use crate::components::sidebar::nav_item::SidebarSectionLabel;
use crate::hooks::{use_app_resource, use_page_visible, use_poll_tick_gated};
use crate::state::overlays::{monitor_overlay_open, open_monitor_overlay, SidebarMonitor};
use crate::state::ui_tasks::{ui_tasks_snapshot, UiTask, UiTaskStatus, UI_TASK_TICK};
use crate::utils::error::AppError;

fn overhead_trigger_label_from(
    result: &Option<Result<probing_proto::prelude::DataFrame, AppError>>,
) -> String {
    match result {
        None => "Loading…".to_string(),
        Some(Err(err)) => {
            table_missing_trigger_label(err).unwrap_or_else(|| "Overhead unavailable".to_string())
        }
        Some(Ok(df)) => overhead_trigger_label(df),
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
pub fn SidebarMonitors() -> Element {
    let overlay_open = monitor_overlay_open();
    let _tick = UI_TASK_TICK.read();
    let tasks = ui_tasks_snapshot();
    let running = tasks.iter().filter(|t| t.is_running()).count();
    let (task_label, task_running) = task_summary_line(&tasks);

    let visible = use_page_visible();
    let poll = use_poll_tick_gated(OVERHEAD_POLL_MS, Some(visible));
    let refresh_tick = poll();

    let summary = use_app_resource(move || {
        let _ = refresh_tick;
        async move { ApiClient::new().fetch_overhead_summary().await }
    });
    let overhead_label = overhead_trigger_label_from(&summary.read());

    rsx! {
        div {
            class: "shrink-0 border-t border-slate-700/30 px-2 py-1.5 space-y-1",
            SidebarSectionLabel { label: "Monitor" }

            button {
                class: "w-full flex items-center gap-1.5 px-2 py-1.5 rounded-md border border-slate-700/50 \
                         bg-slate-800/30 hover:bg-slate-800/60 hover:border-blue-700/40 transition-colors \
                         text-left min-w-0",
                title: "Open background tasks",
                aria_expanded: if overlay_open == Some(SidebarMonitor::Tasks) { "true" } else { "false" },
                onclick: move |e| {
                    e.stop_propagation();
                    open_monitor_overlay(SidebarMonitor::Tasks);
                },
                if task_running {
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
                    "{task_label}"
                }
                if running > 0 {
                    span {
                        class: "shrink-0 text-[10px] tabular-nums text-blue-300 font-medium",
                        "{running}"
                    }
                }
                Icon {
                    icon: &icondata::AiExpandAltOutlined,
                    class: "w-3 h-3 text-slate-500 shrink-0"
                }
            }

            button {
                class: "w-full flex items-center gap-1.5 px-2 py-1.5 rounded-md border border-slate-700/50 \
                         bg-slate-800/30 hover:bg-slate-800/60 hover:border-emerald-700/40 transition-colors \
                         text-left min-w-0",
                title: "Open TorchProbe overhead monitor",
                aria_expanded: if overlay_open == Some(SidebarMonitor::Overhead) { "true" } else { "false" },
                onclick: move |e| {
                    e.stop_propagation();
                    open_monitor_overlay(SidebarMonitor::Overhead);
                },
                Icon {
                    icon: &icondata::CgPerformance,
                    class: "w-3 h-3 text-emerald-400/90 shrink-0"
                }
                span {
                    class: "flex-1 min-w-0 text-[10px] text-slate-200 truncate font-medium",
                    "{overhead_label}"
                }
                Icon {
                    icon: &icondata::AiExpandAltOutlined,
                    class: "w-3 h-3 text-slate-500 shrink-0"
                }
            }
        }
    }
}
