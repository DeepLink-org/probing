//! Dashboard: process info, threads, env vars. Single use_api, content by state.

use dioxus::prelude::*;

use crate::api::ApiClient;
use crate::components::card::Card;
use crate::components::card_view::ThreadsCard;
use crate::components::common::{ErrorState, LoadingState};
use crate::components::data::KeyValueList;
use crate::components::page::{PageContainer, PageTitle};
use crate::hooks::use_api;
use probing_proto::prelude::Process;

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
                subtitle: Some("Process and threads".to_string()),
                icon: Some(&icondata::AiLineChartOutlined),
            }
            {dashboard_content(&state)}
        }
    }
}

fn dashboard_content(state: &crate::hooks::ApiState<Process>) -> Element {
    if state.is_loading() {
        return rsx! {
            Card {
                title: "Loading",
                LoadingState { message: Some("Loading process information...".to_string()) }
            }
        };
    }
    let data = state.data.read();
    if let Some(Err(err)) = data.as_ref() {
        return rsx! {
            Card {
                title: "Error",
                ErrorState { error: err.display_message(), title: None }
            }
        };
    }
    let Some(Ok(process)) = data.as_ref() else {
        return rsx! { div {} };
    };
    rsx! {
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
                        span { class: "font-mono text-sm bg-gray-100 text-gray-900 px-6 py-2 rounded break-all", "{value}" }
                    }
                }
            }
        }
    }
}
