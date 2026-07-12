//! Sidebar monitors — compact task + overhead summary rows; overlays render at app root.

use dioxus::prelude::*;

use crate::api::ApiClient;
use crate::components::icon::Icon;
use crate::components::overhead::{table_missing_trigger_label, OVERHEAD_POLL_MS};
use crate::components::sidebar::nav_item::SidebarSectionLabel;
use crate::hooks::{use_api_with_options, use_page_visible, use_poll_tick_gated, ApiFetchOptions};
use crate::overhead::{df_scalar_f64, OverheadSnapshot};
use crate::state::overlays::{monitor_overlay_open, open_monitor_overlay, SidebarMonitor};
use crate::state::ui_tasks::{ui_tasks_snapshot, UiTask, UiTaskStatus, UI_TASK_TICK};
use crate::utils::error::AppError;

fn overhead_sidebar_from(
    summary: &Option<Result<probing_proto::prelude::DataFrame, AppError>>,
    train_step: &Option<Result<probing_proto::prelude::DataFrame, AppError>>,
) -> crate::overhead::SidebarOverheadCopy {
    match summary {
        None => crate::overhead::SidebarOverheadCopy {
            headline: "Torch overhead".to_string(),
            performance: "Loading…".to_string(),
            overhead: "—".to_string(),
            muted: true,
        },
        Some(Err(err)) => crate::overhead::SidebarOverheadCopy {
            headline: "Torch overhead".to_string(),
            performance: table_missing_trigger_label(err)
                .unwrap_or_else(|| "Unavailable".to_string()),
            overhead: "—".to_string(),
            muted: true,
        },
        Some(Ok(df)) => {
            let train_ms = train_step
                .as_ref()
                .and_then(|r| r.as_ref().ok())
                .and_then(|df| df_scalar_f64(df, "train_step_median_ms", 0));
            OverheadSnapshot::from_summary(df)
                .with_train_step_median(train_ms)
                .sidebar_copy()
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
pub fn SidebarMonitors() -> Element {
    let overlay_open = monitor_overlay_open();
    let _tick = UI_TASK_TICK.read();
    let tasks = ui_tasks_snapshot();
    let running = tasks.iter().filter(|t| t.is_running()).count();
    let (task_label, task_running) = task_summary_line(&tasks);

    let visible = use_page_visible();
    let poll = use_poll_tick_gated(OVERHEAD_POLL_MS, Some(visible));
    let refresh_tick = poll();
    let refresh_opts = ApiFetchOptions {
        keep_previous_while_refreshing: true,
    };

    let summary = use_api_with_options(
        move || {
            let _ = refresh_tick;
            async move { ApiClient::new().fetch_overhead_summary().await }
        },
        refresh_opts,
    );
    let train_step = use_api_with_options(
        move || {
            let _ = refresh_tick;
            async move { ApiClient::new().fetch_overhead_train_step_median().await }
        },
        refresh_opts,
    );

    let overhead = overhead_sidebar_from(
        &summary.data.read().clone(),
        &train_step.data.read().clone(),
    );
    let oh_muted = if overhead.muted {
        "text-slate-400"
    } else {
        "text-emerald-300"
    };

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
                class: "w-full flex flex-col gap-0.5 px-2 py-1.5 rounded-md border border-slate-700/50 \
                         bg-slate-800/30 hover:bg-slate-800/60 hover:border-emerald-700/40 transition-colors \
                         text-left min-w-0",
                title: "Open TorchProbe overhead — step time & hook cost",
                aria_expanded: if overlay_open == Some(SidebarMonitor::Overhead) { "true" } else { "false" },
                onclick: move |e| {
                    e.stop_propagation();
                    open_monitor_overlay(SidebarMonitor::Overhead);
                },
                div { class: "flex items-center gap-1.5 min-w-0",
                    Icon {
                        icon: &icondata::CgPerformance,
                        class: "w-3 h-3 text-emerald-400/90 shrink-0"
                    }
                    span {
                        class: "flex-1 min-w-0 text-[10px] text-slate-100 font-semibold truncate tabular-nums",
                        "{overhead.headline}"
                    }
                    Icon {
                        icon: &icondata::AiExpandAltOutlined,
                        class: "w-3 h-3 text-slate-500 shrink-0"
                    }
                }
                span {
                    class: "text-[9px] text-slate-400 truncate pl-[18px]",
                    "{overhead.performance}"
                }
                span {
                    class: "text-[9px] font-medium truncate pl-[18px] tabular-nums {oh_muted}",
                    "{overhead.overhead}"
                }
            }
        }
    }
}
