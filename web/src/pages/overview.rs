use dioxus::prelude::*;
use probing_proto::prelude::Process;

use crate::components::card::Card;
use crate::components::card_view::ThreadsCard;
use crate::components::data::KeyValueList;
use crate::components::page::{PageContainer, PageHeader};
use crate::components::common::{LoadingState, ErrorState};
use crate::hooks::use_api_simple;
use crate::api::ApiClient;
use crate::styles::{combinations::*, styles::*};

#[component]
pub fn Overview() -> Element {
    let state = use_api_simple::<Process>();
    
    use_effect(move || {
        let mut loading = state.loading.clone();
        let mut data = state.data.clone();
        spawn(async move {
            loading.set(true);
            let client = ApiClient::new();
            data.set(Some(client.get_overview().await));
            loading.set(false);
        });
    });

    rsx! {
        PageContainer {
            PageHeader {
                title: "System Overview".to_string(),
                subtitle: None
            }
            
            if state.is_loading() {
                Card {
                    title: "Loading",
                    LoadingState { message: Some("Loading process information...".to_string()) }
                }
            } else if let Some(Ok(process)) = state.data.read().as_ref() {
                div {
                    class: SPACE_Y_6,
                    Card {
                        title: "Process Information",
                        KeyValueList {
                            items: vec![
                                ("Process ID (PID):", process.pid.to_string()),
                                ("Executable Path:", process.exe.clone()),
                                ("Command Line:", process.cmd.clone()),
                                ("Working Directory:", process.cwd.clone()),
                            ]
                        }
                    }
                    Card {
                        title: "Threads Information",
                        div {
                            class: SPACE_Y_3,
                            div { class: SECTION_SUBTITLE, "Total threads: {process.threads.len()}" }
                            ThreadsCard { threads: process.threads.clone() }
                        }
                    }
                    Card {
                        title: "Environment Variables",
                        EnvVars { env: process.env.clone() }
                    }
                }
            } else if let Some(Err(err)) = state.data.read().as_ref() {
                Card {
                    title: "Error",
                    ErrorState { error: format!("{:?}", err), title: None }
                }
            }
        }
    }
}

#[component]
fn EnvVars(env: std::collections::HashMap<String, String>) -> Element {
    rsx! {
        div {
            class: SPACE_Y_3,
            div { class: format!("{} {}", TEXT_SM, TEXT_GRAY_600), "Total environment variables: {env.len()}" }
            div {
                class: SPACE_Y_2,
                for (name, value) in env {
                    div {
                        class: "flex justify-between items-start py-2 border-b border-gray-200 last:border-b-0",
                        span { class: format!("{} {} {} {}", FONT_MEDIUM, TEXT_GRAY_700, FONT_MONO, TEXT_SM), "{name}" }
                        span { class: format!("{} {} {} {} {} {} {}", FONT_MONO, TEXT_SM, BG_GRAY_100, PX_6, PY_2, ROUNDED, BREAK_ALL), "{value}" }
                    }
                }
            }
        }
    }
}