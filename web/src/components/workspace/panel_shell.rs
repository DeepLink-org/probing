//! Right-side / auxiliary panel shell (header, toolbar, scroll body, footer).

use dioxus::prelude::*;

use crate::components::workspace::surface::SurfaceIconHeader;

#[component]
pub fn WorkspacePanelShell(
    title: String,
    subtitle: Option<String>,
    icon: &'static icondata::Icon,
    #[props(optional)] header_actions: Option<Element>,
    #[props(optional)] toolbar: Option<Element>,
    footer: Element,
    children: Element,
    #[props(default = false)]
    embedded: bool,
) -> Element {
    let shell_class = if embedded {
        "h-full min-h-0 flex flex-col rounded-lg border border-gray-200 bg-white shadow-sm"
    } else {
        "h-full min-h-0 flex flex-col border-l border-gray-200 bg-white"
    };
    rsx! {
        aside {
            class: "{shell_class}",
            SurfaceIconHeader {
                icon: icon,
                icon_class: "w-5 h-5 text-blue-600",
                title: title,
                subtitle: subtitle,
                header_right: header_actions,
            }
            if let Some(bar) = toolbar {
                div { class: "px-3 py-2 border-b border-gray-100 shrink-0", {bar} }
            }
            div {
                class: "flex-1 overflow-y-auto px-3 py-3 space-y-3 min-h-0",
                {children}
            }
            div { class: "shrink-0 p-3 border-t border-gray-200 bg-gray-50/80", {footer} }
        }
    }
}
