use dioxus::prelude::*;
use probing_proto::prelude::CallFrame;

use crate::components::card::Card;
use crate::components::callstack_view::CallStackView;
use crate::components::page::{PageContainer, PageTitle};
use crate::components::common::{LoadingState, ErrorState};
use crate::hooks::use_api;
use crate::api::ApiClient;

#[component]
pub fn Stack(tid: Option<String>) -> Element {
    let tid_display = tid.clone();
    let mut mode = use_signal(|| String::from("mixed")); // py | cpp | mixed

    let state = use_api(move || {
        let tid_clone = tid.clone();
        let current_mode = mode.read().clone();
        let client = ApiClient::new();
        async move {
            client.get_callstack_with_mode(tid_clone, &current_mode).await
        }
    });

    rsx! {
        PageContainer {
            PageTitle {
                title: "Stacks".to_string(),
                subtitle: tid_display.as_ref().map(|t| format!("Call stack for thread: {t}")),
                icon: Some(&icondata::AiThunderboltOutlined),
            }
            Card {
                title: "Call Stack Information",
                header_right: Some(rsx! {
                    div { class: "flex gap-2 items-center",
                        span { class: "text-sm text-gray-600", "Mode:" }
                        button { class: format!("px-3 py-1 rounded transition-colors {}", if mode.read().as_str()=="py" { "bg-indigo-600 text-white shadow-sm" } else { "bg-gray-100 hover:bg-gray-200" }),
                            onclick: move |_| {
                                *mode.write() = String::from("py");
                            }, "Py" }
                        button { class: format!("px-3 py-1 rounded transition-colors {}", if mode.read().as_str()=="cpp" { "bg-indigo-600 text-white shadow-sm" } else { "bg-gray-100 hover:bg-gray-200" }),
                            onclick: move |_| {
                                *mode.write() = String::from("cpp");
                            }, "C++" }
                        button { class: format!("px-3 py-1 rounded transition-colors {}", if mode.read().as_str()=="mixed" { "bg-indigo-600 text-white shadow-sm" } else { "bg-gray-100 hover:bg-gray-200" }),
                            onclick: move |_| {
                                *mode.write() = String::from("mixed");
                            }, "Mixed" }
                    }
                }),
                if state.is_loading() {
                    LoadingState { message: Some("Loading call stack information...".to_string()) }
                } else if let Some(Ok(callframes)) = state.data.read().as_ref() {
                    div {
                        class: "space-y-4",
                        div { class: "text-sm text-gray-600", "Total call frames: {callframes.len()}" }
                        if callframes.is_empty() {
                            div { class: "text-center py-8 text-gray-500", "No call stack data available" }
                        } else {
                            div {
                                class: "space-y-2",
                                {
                                    let current_mode = mode.read().clone();
                                    callframes.iter()
                                        .filter(move |cf| match (current_mode.as_str(), cf) {
                                            ("py", CallFrame::PyFrame { .. }) => true,
                                            ("cpp", CallFrame::CFrame { .. }) => true,
                                            ("mixed", _) => true,
                                            _ => false,
                                        })
                                        .map(|cf| rsx! { CallStackView { callstack: cf.clone() } })
                                }
                            }
                        }
                    }
                } else if let Some(Err(err)) = state.data.read().as_ref() {
                    ErrorState { error: format!("{:?}", err), title: None }
                }
            }
        }
    }
}
