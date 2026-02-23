//! App shell: sidebar (or show-sidebar button when collapsed) + main content area.
//! All page content is rendered inside the main area with consistent padding and max-width.

use dioxus::prelude::*;

use crate::components::icon::Icon;
use crate::components::sidebar::Sidebar;
use crate::state::sidebar::{save_sidebar_state, SIDEBAR_HIDDEN, SIDEBAR_WIDTH};

/// Floating button shown when sidebar is hidden. Kept as a const for clarity and reuse.
const SHOW_SIDEBAR_BUTTON_CLASS: &str = "fixed top-4 left-4 z-50 w-10 h-10 bg-white border border-gray-300 rounded-lg shadow-sm flex items-center justify-center hover:bg-gray-50 focus:outline-none focus:ring-2 focus:ring-blue-500 focus:ring-offset-2";

#[component]
pub fn AppLayout(children: Element) -> Element {
    let _sidebar_width = SIDEBAR_WIDTH.read();
    let sidebar_hidden = SIDEBAR_HIDDEN.read();

    rsx! {
        div {
            class: "flex h-screen bg-gray-50 overflow-hidden",
            if !*sidebar_hidden {
                Sidebar {}
            } else {
                button {
                    class: SHOW_SIDEBAR_BUTTON_CLASS,
                    title: "Show Sidebar",
                    aria_label: "Show sidebar",
                    onclick: move |_| {
                        *SIDEBAR_HIDDEN.write() = false;
                        save_sidebar_state();
                    },
                    Icon {
                        icon: &icondata::AiMenuUnfoldOutlined,
                        class: "w-5 h-5 text-gray-600"
                    }
                }
            }
            main {
                class: "flex-1 overflow-y-auto p-4 sm:p-6 bg-gray-50",
                style: if *sidebar_hidden { "width: 100%;" } else { "" },
                div {
                    class: "max-w-7xl mx-auto w-full",
                    {children}
                }
            }
        }
    }
}
