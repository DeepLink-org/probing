use dioxus::prelude::*;

use crate::components::icon::Icon;

pub const POLL_MS: u32 = 3000;
pub const PREVIEW_RECORD_LIMIT: usize = 50;

#[derive(Clone, PartialEq)]
pub struct StartTraceDraft {
    pub function: String,
    pub watch: String,
    pub print_to_terminal: bool,
}

#[component]
pub fn PollHint(interval_secs: u32) -> Element {
    rsx! {
        span { class: "text-[11px] text-gray-500 tabular-nums",
            "Auto {interval_secs}s"
        }
    }
}

#[component]
pub fn RefreshButton(onclick: EventHandler<()>) -> Element {
    rsx! {
        button {
            class: "inline-flex items-center gap-1 px-2 py-1.5 text-xs rounded-md border border-gray-300 bg-white hover:bg-gray-50",
            onclick: move |_| onclick.call(()),
            Icon { icon: &icondata::AiReloadOutlined, class: "w-3.5 h-3.5" }
            "Refresh"
        }
    }
}
