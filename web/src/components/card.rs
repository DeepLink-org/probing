use dioxus::prelude::*;
use crate::styles::combinations::*;

#[component]
pub fn Card(title: &'static str, children: Element, content_class: Option<&'static str>) -> Element {
    let content_cls = content_class.unwrap_or(CARD_CONTENT);
    rsx! {
        div {
            class: CARD,
            div {
                class: CARD_HEADER,
                h3 { class: CARD_TITLE, "{title}" }
            }
            div { class: content_cls, {children} }
        }
    }
}