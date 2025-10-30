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
    
    use_effect(move || {
        let tid = tid.clone();
        let mut loading = state.loading.clone();
        let mut data = state.data.clone();
        spawn(async move {
            loading.set(true);
            let client = ApiClient::new();
            data.set(Some(client.get_callstack(tid).await));
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
                                for callframe in callframes {
                                    CallStackView { callstack: callframe.clone() }
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