use dioxus::prelude::*;
use crate::components::card::Card;
use crate::components::page::{PageContainer, PageHeader};

#[component]
pub fn Cluster() -> Element {
    rsx! {
        PageContainer {
            PageHeader {
                title: "Cluster Management".to_string(),
                subtitle: Some("Monitor and manage your cluster nodes".to_string())
            }
            
            div {
                class: "grid grid-cols-1 lg:grid-cols-3 gap-6",
                Card {
                    title: "Cluster Overview",
                    StatCard { label: "Total Nodes", value: "8", color: "blue" }
                    StatCard { label: "Active Nodes", value: "7", color: "green" }
                    StatCard { label: "Failed Nodes", value: "1", color: "red" }
                }
                
                Card {
                    title: "Resource Usage",
                    UsageBar { label: "CPU Usage", value: 45 }
                    UsageBar { label: "Memory Usage", value: 67 }
                }
                
                Card {
                    title: "Network Status",
                    StatCard { label: "Latency", value: "12ms", color: "gray" }
                    StatCard { label: "Throughput", value: "1.2 Gbps", color: "gray" }
                    StatCard { label: "Connections", value: "1,234", color: "gray" }
                }
            }
        }
    }
}

#[component]
fn StatCard(label: &'static str, value: &'static str, color: &'static str) -> Element {
    rsx! {
        div {
            class: "flex justify-between items-center",
            span { class: "text-gray-600", "{label}" }
            span {
                class: match color {
                    "blue" => "text-2xl font-bold text-blue-600",
                    "green" => "text-2xl font-bold text-green-600",
                    "red" => "text-2xl font-bold text-red-600",
                    _ => "text-2xl font-bold text-gray-600",
                },
                "{value}"
            }
        }
    }
}

#[component]
fn UsageBar(label: &'static str, value: u32) -> Element {
    rsx! {
        div {
            class: "space-y-2",
            div {
                class: "flex justify-between text-sm",
                span { "{label}" }
                span { "{value}%" }
            }
            div {
                class: "w-full bg-gray-200 rounded-full h-2",
                div {
                    class: "bg-blue-600 h-2 rounded-full",
                    style: "width: {value}%"
                }
            }
        }
    }
}