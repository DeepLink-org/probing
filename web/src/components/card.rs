//! White card with title bar and optional header actions (e.g. filters, mode toggles).

use dioxus::prelude::*;

/// Card with a title row and optional `header_right` slot for buttons or controls.
#[component]
pub fn Card(
    title: &'static str,
    children: Element,
    content_class: Option<&'static str>,
    #[props(optional)] header_right: Option<Element>,
) -> Element {
    let content_cls = content_class.unwrap_or("p-4");
    rsx! {
        div {
            class: "bg-white rounded-lg border border-gray-200",
            div {
                class: "px-4 py-3 border-b border-gray-200",
                div { class: "flex items-center justify-between gap-3",
                    h3 { class: "text-base font-semibold text-gray-900", "{title}" }
                    if let Some(el) = header_right { div { class: "flex items-center gap-2", {el} } }
                }
            }
            div { class: content_cls, {children} }
        }
    }
}
