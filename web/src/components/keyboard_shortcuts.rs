//! Global keyboard shortcuts: ⌘K command palette and ? help overlay.

use dioxus::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

use crate::components::icon::Icon;
use crate::state::agent::AGENT_PANEL_OPEN;
use crate::state::commands::{COMMAND_PANEL_OPEN, SHORTCUTS_HELP_OPEN};
use crate::state::source_viewer::{close_source_viewer, SOURCE_VIEWER_OPEN};

#[component]
pub fn GlobalShortcutInstaller() -> Element {
    let slot = use_hook(|| {
        Rc::new(RefCell::new(
            None::<(
                web_sys::Window,
                Closure<dyn FnMut(web_sys::KeyboardEvent)>,
            )>,
        ))
    });

    let slot_for_effect = slot.clone();
    use_effect(move || {
        if let Some((window, handler)) = slot_for_effect.borrow_mut().take() {
            let listener = handler.as_ref().unchecked_ref();
            let _ = window.remove_event_listener_with_callback("keydown", listener);
        }

        let Some(window) = web_sys::window() else {
            return;
        };

        let handler = Closure::wrap(Box::new(move |e: web_sys::KeyboardEvent| {
            if handle_global_key(&e) {
                e.prevent_default();
            }
        }) as Box<dyn FnMut(web_sys::KeyboardEvent)>);

        let listener = handler.as_ref().unchecked_ref();
        let _ = window.add_event_listener_with_callback("keydown", listener);
        *slot_for_effect.borrow_mut() = Some((window, handler));
    });

    let slot_for_drop = slot.clone();
    use_drop(move || {
        if let Some((window, handler)) = slot_for_drop.borrow_mut().take() {
            let listener = handler.as_ref().unchecked_ref();
            let _ = window.remove_event_listener_with_callback("keydown", listener);
        }
    });

    rsx! {}
}

fn handle_global_key(e: &web_sys::KeyboardEvent) -> bool {
    if e.key() == "Escape" {
        if *SHORTCUTS_HELP_OPEN.read() {
            *SHORTCUTS_HELP_OPEN.write() = false;
            return true;
        }
        if *COMMAND_PANEL_OPEN.read() {
            *COMMAND_PANEL_OPEN.write() = false;
            return true;
        }
        if *AGENT_PANEL_OPEN.read() {
            *AGENT_PANEL_OPEN.write() = false;
            return true;
        }
        if *SOURCE_VIEWER_OPEN.read() {
            close_source_viewer();
            return true;
        }
        return false;
    }

    let mod_key = e.meta_key() || e.ctrl_key();

    if mod_key && e.key().eq_ignore_ascii_case("k") {
        *COMMAND_PANEL_OPEN.write() = true;
        *SHORTCUTS_HELP_OPEN.write() = false;
        return true;
    }

    if mod_key && e.key().eq_ignore_ascii_case("j") {
        *AGENT_PANEL_OPEN.write() = !*AGENT_PANEL_OPEN.read();
        *SHORTCUTS_HELP_OPEN.write() = false;
        return true;
    }

    if e.key() == "?" && !text_input_focused() {
        *SHORTCUTS_HELP_OPEN.write() = !*SHORTCUTS_HELP_OPEN.read();
        return true;
    }

    false
}

fn text_input_focused() -> bool {
    let Some(document) = web_sys::window().and_then(|w| w.document()) else {
        return true;
    };
    let Some(active) = document.active_element() else {
        return false;
    };
    let tag = active.tag_name();
    tag == "INPUT" || tag == "TEXTAREA" || tag == "SELECT"
}

#[component]
pub fn ShortcutsHelpOverlay() -> Element {
    if !*SHORTCUTS_HELP_OPEN.read() {
        return rsx! {};
    }

    rsx! {
        div {
            class: "fixed inset-0 z-[9999] flex items-start justify-center pt-[12vh] bg-black/30 backdrop-blur-sm",
            onclick: move |_| *SHORTCUTS_HELP_OPEN.write() = false,
            div {
                class: "w-full max-w-lg mx-4 bg-white rounded-xl shadow-2xl border border-gray-200 overflow-hidden",
                onclick: move |e| e.stop_propagation(),
                div { class: "flex items-center justify-between px-5 py-4 border-b border-gray-100 bg-gray-50/80",
                    div { class: "flex items-center gap-2",
                        Icon { icon: &icondata::AiThunderboltOutlined, class: "w-4 h-4 text-blue-600" }
                        h2 { class: "text-sm font-semibold text-gray-900", "Keyboard shortcuts" }
                    }
                    button {
                        class: "p-1 rounded-md text-gray-400 hover:text-gray-700 hover:bg-gray-200/80",
                        title: "Close",
                        onclick: move |_| *SHORTCUTS_HELP_OPEN.write() = false,
                        Icon { icon: &icondata::AiCloseOutlined, class: "w-4 h-4" }
                    }
                }
                div { class: "px-5 py-4 max-h-[70vh] overflow-y-auto space-y-4 text-sm",
                    ShortcutSection {
                        title: "Global",
                        items: &[
                            ("⌘K / Ctrl+K", "Open command palette"),
                            ("⌘J / Ctrl+J", "Toggle Investigate overlay (diagnostic agent)"),
                            ("?", "Toggle this help"),
                            ("Esc", "Close palette / investigate / help / source preview"),
                        ],
                    }
                    ShortcutSection {
                        title: "Timeline viewer",
                        items: &[
                            ("W / S", "Zoom in / out"),
                            ("A / D", "Pan earlier / later"),
                            ("F", "Fit entire trace"),
                            ("Z", "Zoom to selection"),
                            ("Esc", "Back / close inspector"),
                        ],
                    }
                    ShortcutSection {
                        title: "Analytics SQL",
                        items: &[
                            ("⌘Enter / Ctrl+Enter", "Run query"),
                            ("Esc", "Close preview modal"),
                        ],
                    }
                    ShortcutSection {
                        title: "Python dialogs",
                        items: &[("Esc", "Close dialog")],
                    }
                }
                div { class: "px-5 py-3 border-t border-gray-100 bg-gray-50/60 text-xs text-gray-500",
                    "Press ? again or Esc to close"
                }
            }
        }
    }
}

#[component]
fn ShortcutSection(title: &'static str, items: &'static [(&'static str, &'static str)]) -> Element {
    rsx! {
        div {
            p { class: "text-[10px] font-semibold uppercase tracking-wide text-gray-400 mb-2", "{title}" }
            div { class: "rounded-lg border border-gray-200 divide-y divide-gray-100 overflow-hidden",
                for (keys, desc) in items {
                    div { class: "flex items-center justify-between gap-4 px-3 py-2 bg-white",
                        span { class: "text-xs text-gray-600", "{desc}" }
                        kbd { class: "shrink-0 px-2 py-0.5 rounded border border-gray-200 bg-gray-50 text-[11px] font-mono text-gray-800",
                            "{keys}"
                        }
                    }
                }
            }
        }
    }
}
