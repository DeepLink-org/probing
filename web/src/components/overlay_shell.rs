//! Centered modal shell for viewport overlays.

use dioxus::prelude::*;

use crate::components::icon::Icon;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum OverlayAccent {
    Blue,
    Emerald,
}

impl OverlayAccent {
    fn header_gradient(self) -> &'static str {
        match self {
            Self::Blue => "bg-gradient-to-r from-blue-50/80 to-white",
            Self::Emerald => "bg-gradient-to-r from-emerald-50/80 to-white",
        }
    }

    fn icon_wrap(self) -> &'static str {
        match self {
            Self::Blue => "bg-blue-100 text-blue-700",
            Self::Emerald => "bg-emerald-100 text-emerald-700",
        }
    }
}

/// Full-viewport centered dialog (mounted at app root).
#[component]
pub fn OverlayShell(
    title: String,
    subtitle: String,
    accent: OverlayAccent,
    close_label: String,
    on_close: EventHandler<()>,
    header_icon: Element,
    header_actions: Element,
    children: Element,
    #[props(default = "max-w-5xl")] max_width: &'static str,
) -> Element {
    let header_gradient = accent.header_gradient();
    let icon_wrap = accent.icon_wrap();

    rsx! {
        div {
            class: "app-overlay fixed inset-0 z-[9999] flex items-center justify-center p-4 sm:p-8 bg-black/55",
            role: "presentation",
            tabindex: "-1",
            onkeydown: move |e: KeyboardEvent| {
                if e.key() == Key::Escape {
                    on_close.call(());
                }
            },
            onclick: move |_| on_close.call(()),
            div {
                class: "relative flex flex-col w-full {max_width} max-h-[min(90vh,920px)] \
                         rounded-xl border border-gray-200 bg-white shadow-2xl overflow-hidden",
                role: "dialog",
                aria_modal: "true",
                aria_label: "{title}",
                onclick: move |e| e.stop_propagation(),
                div {
                    class: "flex items-center gap-3 px-4 sm:px-6 py-4 border-b border-gray-200 {header_gradient} shrink-0",
                    div {
                        class: "flex items-center justify-center w-9 h-9 rounded-lg {icon_wrap} shrink-0",
                        {header_icon}
                    }
                    div { class: "flex-1 min-w-0",
                        h2 { class: "text-base font-semibold text-gray-900 truncate", "{title}" }
                        p { class: "text-xs text-gray-500 mt-0.5", "{subtitle}" }
                    }
                    {header_actions}
                    button {
                        r#type: "button",
                        class: "p-2 rounded-lg border border-gray-200 text-gray-500 hover:bg-gray-100 hover:text-gray-800 transition-colors shrink-0",
                        title: "Close",
                        aria_label: "{close_label}",
                        onclick: move |e| {
                            e.stop_propagation();
                            on_close.call(());
                        },
                        Icon { icon: &icondata::AiCloseOutlined, class: "w-4 h-4" }
                    }
                }
                div {
                    class: "flex-1 overflow-y-auto min-h-0 px-4 sm:px-6 py-4 sm:py-5",
                    {children}
                }
            }
        }
    }
}
