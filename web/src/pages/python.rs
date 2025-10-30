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
            
            div {
                class: "grid grid-cols-1 lg:grid-cols-2 gap-6",
                Card {
                    title: "Python Process Info",
                    InfoRow { label: "Python Version", value: "3.9.7" }
                    InfoRow { label: "Script Path", value: "/app/main.py" }
                    InfoRow { label: "Working Directory", value: "/app" }
                }
                
                Card {
                    title: "Python Modules",
                    ModuleRow { name: "numpy", version: "1.21.0" }
                    ModuleRow { name: "pandas", version: "1.3.0" }
                    ModuleRow { name: "matplotlib", version: "3.4.0" }
                }
            }
        }
    }
}

#[component]
fn InfoRow(label: &'static str, value: &'static str) -> Element {
    rsx! {
        div {
            class: "flex justify-between items-center py-2 border-b border-gray-200 dark:border-gray-700 last:border-b-0",
            span { class: "text-gray-600 dark:text-gray-400", "{label}" }
            span { class: "font-mono text-sm", "{value}" }
        }
    }
}

#[component]
fn ModuleRow(name: &'static str, version: &'static str) -> Element {
    rsx! {
        div {
            class: "flex justify-between items-center py-1",
            span { class: "text-sm", "{name}" }
            span { class: "text-xs text-gray-500", "{version}" }
        }
    }
}