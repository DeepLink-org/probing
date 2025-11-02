use dioxus::prelude::*;
use crate::components::card::Card;
use crate::components::page::{PageContainer, PageHeader};

#[component]
pub fn Python() -> Element {
    rsx! {
        PageContainer {
            PageHeader {
                title: "Python Inspection".to_string(),
                subtitle: Some("Inspect and debug Python processes".to_string())
            }
            
            Card {
                title: "Coming Soon",
                div {
                    class: "text-center py-12",
                    p {
                        class: "text-gray-600 text-lg mb-4",
                        "Python inspection features are coming soon."
                    }
                    p {
                        class: "text-gray-500 text-sm",
                        "This page will display Python process information, modules, and debugging tools."
                    }
                }
            }
        }
    }
}