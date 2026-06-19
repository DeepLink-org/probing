//! Shared UI for async and empty states: loading spinner, error block, empty message.

use dioxus::prelude::*;

use crate::components::colors::colors;
use crate::utils::error::AppError;

/// Centered spinner and optional message. Use while data is loading.
#[component]
pub fn LoadingState(message: Option<String>) -> Element {
    rsx! {
        div {
            class: "flex flex-col items-center justify-center py-12 gap-3",
            div {
                class: "w-8 h-8 border-2 border-gray-300 border-t-blue-600 rounded-full animate-spin",
            }
            div {
                class: "text-sm text-gray-500",
                if let Some(msg) = message {
                    "{msg}"
                } else {
                    "Loading..."
                }
            }
        }
    }
}

/// Error block with optional title. Use when a request or operation fails.
#[component]
pub fn ErrorState(error: String, title: Option<String>) -> Element {
    let class_str = format!(
        "p-4 rounded border text-{} bg-{} border-{}",
        colors::ERROR_TEXT,
        colors::ERROR_LIGHT,
        colors::ERROR_BORDER
    );
    rsx! {
        div {
            class: "{class_str}",
            if let Some(title) = title {
                h3 { class: "font-semibold mb-2", "{title}" }
            }
            pre { class: "text-sm whitespace-pre-wrap break-words", "{error}" }
        }
    }
}

/// Centered message when there is no data to show.
#[component]
pub fn EmptyState(message: String) -> Element {
    rsx! {
        div {
            class: "text-center py-8 text-gray-500",
            "{message}"
        }
    }
}

/// Wraps [`SuspenseBoundary`] with the shared [`LoadingState`] spinner.
#[component]
pub fn AsyncBoundary(
    #[props(optional)] message: Option<String>,
    children: Element,
) -> Element {
    let msg = message;
    rsx! {
        SuspenseBoundary {
            fallback: move |_| rsx! {
                LoadingState { message: msg.clone() }
            },
            {children}
        }
    }
}

/// Render an [`AppError`] after a resource has suspended.
#[component]
pub fn AppErrorDisplay(error: AppError, #[props(optional)] title: Option<String>) -> Element {
    rsx! {
        ErrorState {
            error: error.display_message(),
            title,
        }
    }
}

/// Match a resolved API [`Result`] into error, empty, or success UI.
pub fn query_result<T>(
    result: Result<T, AppError>,
    is_empty: impl FnOnce(&T) -> bool,
    empty_message: &str,
    render: impl FnOnce(T) -> Element,
) -> Element {
    match result {
        Ok(value) if is_empty(&value) => rsx! {
            EmptyState { message: empty_message.to_string() }
        },
        Ok(value) => render(value),
        Err(err) => rsx! {
            AppErrorDisplay { error: err, title: None }
        },
    }
}
