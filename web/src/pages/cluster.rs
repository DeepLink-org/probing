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
            
            Card {
                title: "Coming Soon",
                div {
                    class: "text-center py-12",
                    p {
                        class: "text-gray-600 text-lg mb-4",
                        "Cluster management features are coming soon."
                    }
                    p {
                        class: "text-gray-500 text-sm",
                        "This page will display cluster nodes, resource usage, and network status."
                    }
                }
            }
        }
    }
}