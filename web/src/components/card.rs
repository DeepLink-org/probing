use dioxus::prelude::*;
use crate::styles::combinations::*;

#[component]
pub fn Card(
    title: &'static str,
    children: Element,
    content_class: Option<&'static str>,
    #[props(optional)] header_right: Option<Element>,
) -> Element {
    let content_cls = content_class.unwrap_or(CARD_CONTENT);
    rsx! {
        div {
            class: CARD,
            div {
                class: CARD_HEADER,
                div { class: "flex items-center justify-between gap-3",
                    h3 { class: CARD_TITLE, "{title}" }
                    if let Some(el) = header_right { div { class: "flex items-center gap-2", {el} } }
                }
            }
            div { class: content_cls, {children} }
        }
    }
}