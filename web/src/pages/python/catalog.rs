use dioxus::prelude::*;

use crate::api::{ApiClient, TraceableItem};
use crate::components::colors::colors;
use crate::components::common::{query_result, EmptyState};
use crate::components::icon::Icon;
use crate::hooks::use_app_resource;

#[component]
pub fn TraceableCatalog(on_start: EventHandler<(String, Vec<String>)>) -> Element {
    let mut filter = use_signal(String::new);
    let items = use_app_resource(|| async move { ApiClient::new().get_traceable_items(None).await });
    let list = items.suspend()?();

    query_result(
        list,
        |items| items.is_empty(),
        "No traceable functions found in the target process.",
        move |items| {
            let filtered = filter_traceables(&items, &filter());
            rsx! {
                div { class: "border-b border-gray-200 px-3 py-2.5 bg-gray-50/80",
                    div { class: "relative",
                        span { class: "absolute left-2.5 top-1/2 -translate-y-1/2 text-gray-400 pointer-events-none",
                            Icon { icon: &icondata::AiSearchOutlined, class: "w-4 h-4" }
                        }
                        input {
                            r#type: "text",
                            class: "w-full pl-8 pr-3 py-2 text-sm rounded-md border border-gray-300 bg-white focus:outline-none focus:ring-2 focus:ring-blue-500/30 focus:border-blue-500",
                            placeholder: "Filter module or function…",
                            value: "{filter}",
                            oninput: move |ev| filter.set(ev.value()),
                        }
                    }
                    p { class: "mt-1.5 text-xs text-gray-500",
                        "{filtered.len()} of {items.len()} items · expand modules · Trace or pick variables"
                    }
                }
                if filtered.is_empty() {
                    div { class: "px-4 py-10",
                        EmptyState { message: format!("No items match \"{}\"", filter()) }
                    }
                } else {
                    div { class: "p-2 space-y-1 max-h-[calc(100vh-14rem)] overflow-y-auto",
                        for item in filtered {
                            TraceableRow {
                                key: "{item.name}",
                                item: item.clone(),
                                on_start,
                            }
                        }
                    }
                }
            }
        },
    )
}

fn filter_traceables(items: &[TraceableItem], query: &str) -> Vec<TraceableItem> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return items.to_vec();
    }
    items
        .iter()
        .filter(|item| {
            item.name.to_lowercase().contains(&q)
                || item
                    .variables
                    .iter()
                    .any(|v| v.to_lowercase().contains(&q))
        })
        .cloned()
        .collect()
}

#[component]
fn TraceableRow(
    item: TraceableItem,
    on_start: EventHandler<(String, Vec<String>)>,
) -> Element {
    let is_module = item.item_type == "M";
    let mut expanded = use_signal(|| false);

    rsx! {
        div { class: "rounded-lg border border-gray-200 bg-white overflow-hidden",
            div { class: "flex items-start gap-2 px-3 py-2.5",
                if is_module {
                    button {
                        class: "mt-0.5 shrink-0 p-0.5 rounded text-gray-400 hover:text-gray-700 hover:bg-gray-100",
                        onclick: move |_| expanded.set(!expanded()),
                        if expanded() {
                            Icon { icon: &icondata::AiCaretDownOutlined, class: "w-3.5 h-3.5" }
                        } else {
                            Icon { icon: &icondata::AiCaretRightOutlined, class: "w-3.5 h-3.5" }
                        }
                    }
                } else {
                    span { class: "w-4 shrink-0" }
                }
                div { class: "min-w-0 flex-1 space-y-2",
                    div { class: "flex flex-wrap items-center gap-2",
                        TypeBadge { item_type: item.item_type.clone() }
                        span { class: "font-mono text-sm text-gray-900 break-all", "{item.name}" }
                        if !is_module {
                            button {
                                class: format!(
                                    "ml-auto shrink-0 px-2.5 py-1 text-xs rounded-md text-white bg-{} hover:bg-{}",
                                    colors::PRIMARY,
                                    colors::PRIMARY_HOVER,
                                ),
                                onclick: {
                                    let name = item.name.clone();
                                    let vars = item.variables.clone();
                                    move |_| on_start.call((name.clone(), vars.clone()))
                                },
                                "Trace"
                            }
                        }
                    }
                    if !is_module && !item.variables.is_empty() {
                        div { class: "flex flex-wrap gap-1.5",
                            for var in item.variables.iter() {
                                {
                                    let v = var.clone();
                                    let name = item.name.clone();
                                    rsx! {
                                        button {
                                            class: format!(
                                                "text-[11px] px-2 py-0.5 rounded border bg-{} text-{} border-{} hover:bg-blue-100 font-mono",
                                                colors::CONTENT_ACCENT_BG,
                                                colors::CONTENT_ACCENT_TEXT,
                                                colors::CONTENT_ACCENT_BORDER,
                                            ),
                                            title: "Start tracing this variable",
                                            onclick: move |_| on_start.call((name.clone(), vec![v.clone()])),
                                            "{v}"
                                        }
                                    }
                                }
                            }
                        }
                    } else if !is_module {
                        p { class: "text-[11px] text-gray-400", "No traceable locals reported" }
                    }
                }
            }
            if is_module && expanded() {
                ModuleChildren {
                    prefix: item.name.clone(),
                    on_start,
                }
            }
        }
    }
}

#[component]
fn ModuleChildren(
    prefix: String,
    on_start: EventHandler<(String, Vec<String>)>,
) -> Element {
    let children = use_app_resource(move || {
        let p = prefix.clone();
        async move { ApiClient::new().get_traceable_items(Some(&p)).await }
    });
    let list = children.suspend()?();

    query_result(
        list,
        |items| items.is_empty(),
        "Empty module",
        move |items| {
            rsx! {
                div { class: "border-t border-gray-100 bg-gray-50/50 p-2 space-y-1",
                    for child in items {
                        TraceableRow {
                            key: "{child.name}",
                            item: child.clone(),
                            on_start,
                        }
                    }
                }
            }
        },
    )
}

#[component]
fn TypeBadge(item_type: String) -> Element {
    let (label, class) = match item_type.as_str() {
        "F" => ("fn", "bg-blue-50 text-blue-800 border-blue-200"),
        "M" => ("mod", "bg-emerald-50 text-emerald-800 border-emerald-200"),
        other => (other, "bg-gray-100 text-gray-700 border-gray-200"),
    };
    rsx! {
        span {
            class: "shrink-0 text-[10px] font-semibold uppercase tracking-wide px-1.5 py-0.5 rounded border {class}",
            "{label}"
        }
    }
}
