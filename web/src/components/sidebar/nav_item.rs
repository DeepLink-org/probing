//! Sidebar nav link and shared class helper for active/inactive state.

use dioxus::prelude::*;
use dioxus_router::Link;
use icondata::Icon as IconData;

use crate::app::Route;
use crate::components::colors::colors;
use crate::components::icon::Icon;

/// One style for all sidebar items (nav link, Profiling button, sub-items). Single source of truth.
pub fn sidebar_item_class(is_active: bool) -> String {
    if is_active {
        format!(
            "flex items-center gap-2 px-2 py-1.5 text-sm font-medium rounded-md bg-{} text-{} border-l-2 border-{}",
            colors::PRIMARY_BG,
            colors::PRIMARY_TEXT,
            colors::PRIMARY_BORDER
        )
    } else {
        format!(
            "flex items-center gap-2 px-2 py-1.5 text-sm font-medium rounded-md text-{} hover:bg-{} hover:text-{} transition-colors",
            colors::SIDEBAR_TEXT_SECONDARY,
            colors::SIDEBAR_HOVER_BG,
            colors::PRIMARY_TEXT
        )
    }
}

#[component]
pub fn SidebarNavItem(
    to: Route,
    icon: &'static IconData,
    label: &'static str,
    is_active: bool,
) -> Element {
    rsx! {
        Link {
            to: to,
            class: "{sidebar_item_class(is_active)}",
            Icon { icon, class: "w-4 h-4" }
            span { "{label}" }
        }
    }
}
