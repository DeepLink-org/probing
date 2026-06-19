//! Live polling status: last refresh time and pause when the tab is hidden.

use dioxus::prelude::*;

use crate::components::icon::Icon;
use crate::hooks::use_page_visible;

#[component]
pub fn PollStatusBar(interval_secs: u32, poll_tick: u32) -> Element {
    let visible = use_page_visible();
    let paused = !visible();
    let mut last_updated = use_signal(|| None::<String>);

    use_effect(move || {
        let _ = poll_tick;
        if visible() {
            last_updated.set(Some(format_local_time()));
        }
    });

    let status = if paused {
        "Paused (tab in background)".to_string()
    } else if let Some(at) = last_updated.read().clone() {
        format!("Updated {at} · auto every {interval_secs}s")
    } else {
        format!("Auto refresh every {interval_secs}s")
    };

    rsx! {
        span {
            class: "text-[11px] text-gray-500 tabular-nums whitespace-nowrap",
            title: if paused {
                "Polling resumes when you return to this tab"
            } else {
                "Metrics refresh automatically while this tab is visible"
            },
            "{status}"
        }
    }
}

fn format_local_time() -> String {
    let now = js_sys::Date::new_0();
    let hours = now.get_hours();
    let minutes = now.get_minutes();
    let seconds = now.get_seconds();
    format!("{hours:02}:{minutes:02}:{seconds:02}")
}

/// Status line for manually refreshed pages (Cluster, Spans, etc.).
#[component]
pub fn ManualRefreshStatus(refresh_tick: u32) -> Element {
    let mut last_updated = use_signal(|| None::<String>);

    use_effect(move || {
        let _ = refresh_tick;
        last_updated.set(Some(format_local_time()));
    });

    let status = if let Some(at) = last_updated.read().clone() {
        format!("Updated {at}")
    } else {
        "Not loaded yet".to_string()
    };

    rsx! {
        span {
            class: "text-[11px] text-gray-500 tabular-nums whitespace-nowrap",
            "{status}"
        }
    }
}

#[component]
pub fn RefreshButton(onclick: EventHandler<()>) -> Element {
    rsx! {
        button {
            class: "inline-flex items-center gap-1 px-2 py-1.5 text-xs rounded-md border border-gray-300 bg-white hover:bg-gray-50",
            title: "Refresh data",
            onclick: move |_| onclick.call(()),
            Icon { icon: &icondata::AiReloadOutlined, class: "w-3.5 h-3.5" }
            "Refresh"
        }
    }
}
