use dioxus::prelude::*;
// Tailwind classes inlined for list items.

#[component]
pub fn KeyValueList(items: Vec<(&'static str, String)>) -> Element {
    rsx! {
        div {
            class: "space-y-3",
            for (label, value) in items {
                div {
                    class: "flex justify-between items-center py-2 border-b border-gray-200 last:border-b-0",
                    span { class: "font-medium text-gray-700", "{label}" }
                    span { class: "font-mono text-sm bg-gray-100 px-2 py-1 rounded break-all", "{value}" }
                }
            }
        }
    }
}