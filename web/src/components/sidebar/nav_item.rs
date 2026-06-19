//! Sidebar nav link and shared class helper for active/inactive state.

use dioxus::prelude::*;
use dioxus_router::Link;
use icondata::Icon as IconData;

use crate::app::Route;
use crate::components::colors::colors;
use crate::components::icon::Icon;

/// One style for all sidebar items (nav link, Profiling button, sub-items). Single source of truth.
pub fn sidebar_item_class(is_active: bool) -> &'static str {
    if is_active {
        colors::SIDEBAR_ITEM_ACTIVE
    } else {
        colors::SIDEBAR_ITEM_INACTIVE
    }
}

#[component]
pub fn SidebarSectionLabel(label: &'static str) -> Element {
    rsx! {
        div {
            class: "px-2 pt-3 pb-1 text-[10px] font-semibold uppercase tracking-wider text-slate-500 select-none",
            "{label}"
        }
    }
}

#[component]
pub fn SidebarNavItem(
    to: Route,
    icon: &'static IconData,
    label: &'static str,
    is_active: bool,
    #[props(default = "")] title: &'static str,
) -> Element {
    rsx! {
        Link {
            to: to,
            class: "{sidebar_item_class(is_active)}",
            title: if title.is_empty() { "" } else { title },
            Icon { icon, class: "w-4 h-4" }
            span { "{label}" }
        }
    }
}
