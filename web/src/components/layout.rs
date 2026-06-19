//! App shell: sidebar (or show-sidebar button when collapsed) + main content area.
//! All page content is rendered inside the main area with consistent padding and max-width.
//! Command Panel and floating result overlay are rendered in the main area.

use dioxus::prelude::*;

use crate::components::agent::{AgentPanel, LlmSettingsOverlay};
use crate::components::global_command_panel::{CommandBar, FloatingResultToast, GlobalCommandPanel};
use crate::components::icon::Icon;
use crate::components::investigation_context_bar::InvestigationContextBar;
use crate::components::keyboard_shortcuts::{GlobalShortcutInstaller, ShortcutsHelpOverlay};
use crate::components::sidebar::Sidebar;
use crate::state::commands::{FloatingResult, COMMAND_PANEL_OPEN};
use crate::state::investigation::load_investigation_context;
use crate::state::investigation_url::InvestigationUrlSync;
use crate::state::llm_config::load_llm_config;
use crate::state::sidebar::{save_sidebar_state, SIDEBAR_HIDDEN, SIDEBAR_WIDTH};

/// Floating button shown when sidebar is hidden. Kept as a const for clarity and reuse.
const SHOW_SIDEBAR_BUTTON_CLASS: &str = "fixed top-4 left-4 z-50 w-10 h-10 bg-white border border-gray-300 rounded-lg shadow-sm flex items-center justify-center hover:bg-gray-50 focus:outline-none focus:ring-2 focus:ring-blue-500 focus:ring-offset-2";

#[component]
pub fn AppLayout(
    children: Element,
    #[props(default = false)] fullscreen: bool,
) -> Element {
    let _sidebar_width = SIDEBAR_WIDTH.read();
    let sidebar_hidden = SIDEBAR_HIDDEN.read();
    let mut floating_result = use_signal(|| Option::<FloatingResult>::None);

    use_effect(move || {
        load_investigation_context();
        load_llm_config();
    });

    rsx! {
        GlobalShortcutInstaller {}
        InvestigationUrlSync {}
        if *COMMAND_PANEL_OPEN.read() {
            GlobalCommandPanel {}
        }
        ShortcutsHelpOverlay {}
        LlmSettingsOverlay {}
        FloatingResultToast {
            result: floating_result,
        }

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
            div {
                class: "flex-1 flex flex-col min-w-0",
                CommandBar {
                    on_execute_done: move |r| *floating_result.write() = Some(r),
                }
                InvestigationContextBar {}
                div {
                    class: "flex flex-1 min-h-0 overflow-hidden",
                    main {
                        class: "flex-1 overflow-y-auto p-4 sm:p-6 bg-gray-50 min-w-0",
                        style: if *sidebar_hidden { "width: 100%;" } else { "" },
                        if fullscreen {
                            div { class: "w-full h-full min-h-0", {children} }
                        } else {
                            div {
                                class: "max-w-7xl mx-auto w-full",
                                {children}
                            }
                        }
                    }
                    AgentPanel {}
                }
            }
        }
    }
}
