use dioxus::prelude::*;
use crate::styles::combinations::*;

#[component]
pub fn KeyValueList(items: Vec<(&'static str, String)>) -> Element {
    rsx! {
        div {
            class: "space-y-3",
            for (label, value) in items {
                div {
                    class: LIST_ITEM,
                    span { class: LIST_ITEM_LABEL, "{label}" }
                    span { class: LIST_ITEM_VALUE, "{value}" }
                }
            }
        }
    }
}