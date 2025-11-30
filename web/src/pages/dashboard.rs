use dioxus::prelude::*;

use crate::components::card::Card;
use crate::components::card_view::ThreadsCard;
use crate::components::data::KeyValueList;
use crate::components::page::{PageContainer, PageTitle};
use crate::components::common::{LoadingState, ErrorState};
use crate::hooks::use_api;
use crate::api::ApiClient;

#[component]
pub fn Dashboard() -> Element {
    let state = use_api(|| {
        let client = ApiClient::new();
        async move { client.get_overview().await }
    });

    rsx! {
        PageContainer {
            PageTitle {
                title: "Dashboard".to_string(),
                subtitle: Some("System overview and process information".to_string()),
                icon: Some(&icondata::AiLineChartOutlined),
            }
            if state.is_loading() {
                Card {
                    title: "Loading",
                    LoadingState { message: Some("Loading process information...".to_string()) }
                }
            } else if let Some(Ok(process)) = state.data.read().as_ref() {
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
                        class: "space-y-3",
                        div { class: "text-sm text-gray-600", "Total threads: {process.threads.len()}" }
                        ThreadsCard { threads: process.threads.clone() }
                    }
                }
                Card {
                    title: "Environment Variables",
                    EnvVars { env: process.env.clone() }
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
            class: "space-y-3",
            div { class: "text-sm text-gray-600", "Total environment variables: {env.len()}" }
            div {
                class: "space-y-2",
                for (name, value) in env {
                    div {
                        class: "flex justify-between items-start py-2 border-b border-gray-200 last:border-b-0",
                        span { class: "font-medium text-gray-700 font-mono text-sm", "{name}" }
                        span { class: "font-mono text-sm bg-gray-100 px-6 py-2 rounded break-all", "{value}" }
                    }
                }
            }
        }
    }
}
