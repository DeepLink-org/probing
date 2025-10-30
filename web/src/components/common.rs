use dioxus::prelude::*;
use crate::styles::combinations::*;

#[component]
pub fn LoadingState(message: Option<String>) -> Element {
    rsx! {
        div {
            class: LOADING,
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
            class: ERROR,
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
            class: EMPTY,
            "{message}"
        }
    }
}

#[component]
pub fn PageTitle(title: String) -> Element {
    rsx! {
        h1 {
            class: PAGE_TITLE,
            "{title}"
        }
    }
}