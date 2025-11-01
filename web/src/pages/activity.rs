use dioxus::prelude::*;
use probing_proto::prelude::CallFrame;

use crate::components::card::Card;
use crate::components::callstack_view::CallStackView;
use crate::components::page::{PageContainer, PageHeader};
use crate::components::common::{LoadingState, ErrorState};
use crate::hooks::use_api_simple;
use crate::api::ApiClient;
use crate::styles::{combinations::*, styles::*};

#[component]
pub fn Activity(tid: Option<String>) -> Element {
    let tid_display = tid.clone();
    let state = use_api_simple::<Vec<CallFrame>>();
    let mut mode = use_signal(|| String::from("mixed")); // py | cpp | mixed
    
    use_effect(move || {
        let tid = tid.clone();
        let mut loading = state.loading.clone();
        let mut data = state.data.clone();
        let current_mode = mode.read().clone();
        spawn(async move {
            loading.set(true);
            let client = ApiClient::new();
            data.set(Some(client.get_callstack_with_mode(tid, &current_mode).await));
            loading.set(false);
        });
    });

    rsx! {
        PageContainer {
            PageHeader {
                title: "Call Stacks".to_string(),
                subtitle: tid_display.as_ref().map(|t| format!("Call stack for thread: {t}"))
            }
            
            Card {
                title: "Call Stack Information",
                header_right: Some(rsx! {
                    div { class: "flex gap-2 items-center",
                        span { class: "text-sm text-gray-600", "Mode:" }
                        button { class: format!("px-3 py-1 rounded {}", if mode.read().as_str()=="py" { "bg-blue-600 text-white" } else { "bg-gray-100" }),
                            onclick: move |_| mode.set(String::from("py")), "Py" }
                        button { class: format!("px-3 py-1 rounded {}", if mode.read().as_str()=="cpp" { "bg-blue-600 text-white" } else { "bg-gray-100" }),
                            onclick: move |_| mode.set(String::from("cpp")), "C++" }
                        button { class: format!("px-3 py-1 rounded {}", if mode.read().as_str()=="mixed" { "bg-blue-600 text-white" } else { "bg-gray-100" }),
                            onclick: move |_| mode.set(String::from("mixed")), "Mixed" }
                    }
                }),
                if state.is_loading() {
                    LoadingState { message: Some("Loading call stack information...".to_string()) }
                } else if let Some(Ok(callframes)) = state.data.read().as_ref() {
                    div {
                        class: SPACE_Y_4,
                        div { class: SECTION_SUBTITLE, "Total call frames: {callframes.len()}" }
                        if callframes.is_empty() {
                            div { class: format!("{} {} {}", TEXT_CENTER, PY_8, TEXT_GRAY_500), "No call stack data available" }
                        } else {
                            div {
                                class: SPACE_Y_2,
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