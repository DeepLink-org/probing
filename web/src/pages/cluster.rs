use std::time::{Duration, SystemTime};

use chrono::{DateTime, Utc};
use dioxus::prelude::*;
use probing_proto::prelude::*;

use crate::components::card::Card;
use crate::components::page::{PageContainer, PageHeader};
use crate::components::common::{LoadingState, ErrorState, EmptyState};
use crate::hooks::use_api;
use crate::api::ApiClient;

#[component]
pub fn Cluster() -> Element {
    let state = use_api(|| {
        let client = ApiClient::new();
        async move { client.get_nodes().await }
    });

    rsx! {
        PageContainer {
            PageHeader {
                title: "Cluster Management".to_string(),
                subtitle: Some("Monitor and manage your cluster nodes".to_string())
            }
            
            if state.is_loading() {
                Card {
                    title: "Nodes",
                    LoadingState { message: Some("Loading nodes...".to_string()) }
                }
            } else if let Some(Err(error)) = state.data.read().as_ref() {
                Card {
                    title: "Nodes",
                    ErrorState { 
                        error: error.to_string(),
                        title: Some("Failed to load nodes".to_string())
                    }
                }
            } else if let Some(Ok(nodes)) = state.data.read().as_ref() {
                if nodes.is_empty() {
                    Card {
                        title: "Nodes",
                        EmptyState { message: "No nodes found".to_string() }
                    }
                } else {
                    Card {
                        title: "Nodes",
                        div {
                            class: "overflow-x-auto",
                            ClusterTable { nodes: nodes.clone() }
                        }
                    }
                }
            } else {
                Card {
                    title: "Nodes",
                    LoadingState { message: Some("Initializing...".to_string()) }
                }
            }
        }
    }
}

#[component]
fn ClusterTable(nodes: Vec<Node>) -> Element {
    // 预处理节点数据，包括格式化时间戳和URL
    let processed_nodes: Vec<_> = nodes
        .iter()
        .map(|node| {
            let datetime: DateTime<Utc> = (SystemTime::UNIX_EPOCH
                + Duration::from_micros(node.timestamp))
                .into();
            let timestamp_str = datetime.to_rfc3339();
            let url = format!("http://{}", node.addr);
            (node, timestamp_str, url)
        })
        .collect();

    rsx! {
        div {
            class: "w-full overflow-x-auto border border-gray-200 rounded-lg",
            table {
                class: "w-full border-collapse table-auto",
                thead {
                    tr { class: "bg-gray-50 border-b border-gray-200",
                        th { class: "px-4 py-2 text-left font-semibold text-gray-700 border-r border-gray-200", "host" }
                        th { class: "px-4 py-2 text-left font-semibold text-gray-700 border-r border-gray-200", "address" }
                        th { class: "px-4 py-2 text-left font-semibold text-gray-700 border-r border-gray-200", "local_rank" }
                        th { class: "px-4 py-2 text-left font-semibold text-gray-700 border-r border-gray-200", "rank" }
                        th { class: "px-4 py-2 text-left font-semibold text-gray-700 border-r border-gray-200", "world_size" }
                        th { class: "px-4 py-2 text-left font-semibold text-gray-700 border-r border-gray-200", "group_rank" }
                        th { class: "px-4 py-2 text-left font-semibold text-gray-700 border-r border-gray-200", "group_world_size" }
                        th { class: "px-4 py-2 text-left font-semibold text-gray-700 border-r border-gray-200", "role_name" }
                        th { class: "px-4 py-2 text-left font-semibold text-gray-700 border-r border-gray-200", "role_rank" }
                        th { class: "px-4 py-2 text-left font-semibold text-gray-700 border-r border-gray-200", "role_world_size" }
                        th { class: "px-4 py-2 text-left font-semibold text-gray-700 border-r border-gray-200", "status" }
                        th { class: "px-4 py-2 text-left font-semibold text-gray-700 border-r border-gray-200", "timestamp" }
                    }
                }
                tbody {
                    for (idx, (node, timestamp_str, url)) in processed_nodes.iter().enumerate() {
                        tr { 
                            class: if idx % 2 == 0 { "bg-white" } else { "bg-gray-50" },
                            td { class: "px-4 py-2 text-gray-700 border-r border-gray-200", {node.host.clone()} }
                            td { class: "px-4 py-2 text-gray-700 border-r border-gray-200",
                                a {
                                    href: "{url}",
                                    class: "text-blue-600 hover:text-blue-800 hover:underline",
                                    {node.addr.clone()}
                                }
                            }
                            td { class: "px-4 py-2 text-gray-700 border-r border-gray-200", {node.local_rank.unwrap_or(-1).to_string()} }
                            td { class: "px-4 py-2 text-gray-700 border-r border-gray-200", {node.rank.unwrap_or(-1).to_string()} }
                            td { class: "px-4 py-2 text-gray-700 border-r border-gray-200", {node.world_size.unwrap_or(-1).to_string()} }
                            td { class: "px-4 py-2 text-gray-700 border-r border-gray-200", {node.group_rank.unwrap_or(-1).to_string()} }
                            td { class: "px-4 py-2 text-gray-700 border-r border-gray-200", {node.group_world_size.unwrap_or(-1).to_string()} }
                            td { class: "px-4 py-2 text-gray-700 border-r border-gray-200", {node.role_name.clone().unwrap_or_default()} }
                            td { class: "px-4 py-2 text-gray-700 border-r border-gray-200", {node.role_rank.unwrap_or(-1).to_string()} }
                            td { class: "px-4 py-2 text-gray-700 border-r border-gray-200", {node.role_world_size.unwrap_or(-1).to_string()} }
                            td { class: "px-4 py-2 text-gray-700 border-r border-gray-200", {node.status.clone().unwrap_or_default()} }
                            td { class: "px-4 py-2 text-gray-700 border-r border-gray-200", {timestamp_str.clone()} }
                        }
                    }
                }
            }
        }
    }
}
