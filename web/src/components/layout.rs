use dioxus::prelude::*;

use crate::components::sidebar::Sidebar;
use crate::components::icon::Icon;
use crate::app::{SIDEBAR_WIDTH, SIDEBAR_HIDDEN};

#[component]
pub fn AppLayout(children: Element) -> Element {
    let _sidebar_width = SIDEBAR_WIDTH.read();
    let sidebar_hidden = SIDEBAR_HIDDEN.read();

    rsx! {
        div {
            class: "flex h-screen bg-gradient-to-br from-gray-50 to-indigo-50/30 overflow-hidden",
            if !*sidebar_hidden {
                Sidebar {}
            } else {
                button {
                    class: "fixed top-4 left-4 z-50 w-10 h-10 bg-white border border-gray-300 rounded-lg shadow-sm flex items-center justify-center hover:bg-gray-50",
                    title: "Show Sidebar",
                    onclick: move |_| {
                        *SIDEBAR_HIDDEN.write() = false;
                        if let Some(window) = web_sys::window() {
                            let storage = window.local_storage().ok().flatten();
                            if let Some(storage) = storage {
                                let _ = storage.set_item("sidebar_hidden", "false");
                            }
                        }
                    },
                    Icon {
                        icon: &icondata::AiMenuUnfoldOutlined,
                        class: "w-5 h-5 text-gray-600"
                    }
                }
            }
            main {
                class: "flex-1 overflow-y-auto p-6",
                style: if *sidebar_hidden {
                    "width: 100%;"
                } else {
                    ""
                },
                {children}
            }
        }
    }
}
