use dioxus::prelude::*;

#[component]
pub fn LoadingState(message: Option<String>) -> Element {
    rsx! {
        div {
            class: "text-center py-8 text-gray-500",
            if let Some(msg) = message {
                "{msg}"
            } else {
                "Loading..."
            }
        }
    }
}

#[component]
pub fn ErrorState(error: String, title: Option<String>) -> Element {
    rsx! {
        div {
            class: "text-red-500 p-4 bg-red-50 border border-red-200 rounded",
            if let Some(title) = title {
                h3 { class: "font-semibold mb-2", "{title}" }
            }
            "{error}"
        }
    }
}

#[component]
pub fn EmptyState(message: String) -> Element {
    rsx! {
        div {
            class: "text-center py-8 text-gray-500",
            "{message}"
        }
    }
}
