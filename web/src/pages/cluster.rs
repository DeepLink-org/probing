use std::time::{Duration, SystemTime};

use chrono::{DateTime, Utc};
use dioxus::prelude::*;
use probing_proto::prelude::Node;

use crate::api::ApiClient;
use crate::components::card::Card;
use crate::components::colors::colors;
use crate::components::common::{AsyncBoundary, EmptyState, ErrorState};
use crate::components::icon::Icon;
use crate::components::page::{PageContainer, PageTitle};
use crate::components::poll_status::{ManualRefreshStatus, RefreshButton};
use crate::components::stat_card::StatCard;
use crate::hooks::use_app_resource;

#[component]
pub fn Cluster() -> Element {
    let mut refresh = use_signal(|| 0u32);
    let refresh_tick = refresh();
    let nodes = use_app_resource(move || {
        let _ = refresh();
        async move { ApiClient::new().get_nodes().await }
    });

    rsx! {
        PageContainer {
            PageTitle {
                title: "Cluster".to_string(),
                subtitle: Some("Distributed training nodes and health".to_string()),
                icon: Some(&icondata::AiClusterOutlined),
                header_right: Some(rsx! {
                    ManualRefreshStatus { refresh_tick }
                    RefreshButton {
                        onclick: move |_| refresh.set(refresh() + 1),
                    }
                }),
            }
            AsyncBoundary {
                message: Some("Loading cluster nodes…".to_string()),
                ClusterBody { nodes: nodes(), refresh }
            }
        }
    }
}

#[component]
fn ClusterBody(
    nodes: Option<Result<Vec<Node>, crate::utils::error::AppError>>,
    refresh: Signal<u32>,
) -> Element {
    let Some(result) = nodes else {
        return rsx! { div {} };
    };

    match result {
        Err(err) => rsx! {
            Card {
                title: "Nodes",
                ErrorState {
                    title: Some("Failed to load nodes".to_string()),
                    error: err.display_message(),
                }
            }
        },
        Ok(nodes) if nodes.is_empty() => rsx! {
            Card {
                title: "Nodes",
                EmptyState { message: "No cluster nodes registered.".to_string() }
            }
        },
        Ok(nodes) => {
            let total = nodes.len();
            let healthy = nodes
                .iter()
                .filter(|n| node_status_tone(n) == NodeTone::Healthy)
                .count();
            let degraded = nodes
                .iter()
                .filter(|n| node_status_tone(n) == NodeTone::Degraded)
                .count();
            rsx! {
                div { class: "space-y-4",
                    div { class: "flex flex-wrap items-center justify-between gap-2",
                        div { class: "grid grid-cols-2 sm:grid-cols-4 gap-3 flex-1",
                            StatCard { label: "Nodes", value: total.to_string(), hint: None }
                            StatCard {
                                label: "Healthy",
                                value: healthy.to_string(),
                                hint: None,
                            }
                            StatCard {
                                label: "Degraded",
                                value: degraded.to_string(),
                                hint: None,
                            }
                            StatCard {
                                label: "World size",
                                value: nodes
                                    .first()
                                    .and_then(|n| n.world_size)
                                    .map(|w| w.to_string())
                                    .unwrap_or_else(|| "—".to_string()),
                                hint: None,
                            }
                        }
                        button {
                            class: format!(
                                "inline-flex items-center gap-1.5 px-3 py-2 text-xs rounded-md border border-{} text-{} bg-{} hover:bg-{}",
                                colors::CONTENT_ACCENT_BORDER,
                                colors::CONTENT_ACCENT_TEXT,
                                colors::CONTENT_ACCENT_BG,
                                colors::BTN_SECONDARY_HOVER,
                            ),
                            onclick: move |_| refresh.set(refresh() + 1),
                            Icon { icon: &icondata::AiReloadOutlined, class: "w-3.5 h-3.5" }
                            "Refresh nodes"
                        }
                    }
                    Card {
                        title: "Nodes",
                        content_class: Some("p-0"),
                        ClusterTable { nodes }
                    }
                }
            }
        }
    }
}

#[derive(PartialEq)]
enum NodeTone {
    Healthy,
    Degraded,
    Unknown,
}

fn node_status_tone(node: &Node) -> NodeTone {
    match node.status.as_deref().unwrap_or("").to_lowercase().as_str() {
        "ok" | "healthy" | "running" | "ready" | "online" => NodeTone::Healthy,
        "failed" | "error" | "offline" | "unhealthy" => NodeTone::Degraded,
        "" => NodeTone::Unknown,
        _ => NodeTone::Degraded,
    }
}

#[component]
fn ClusterTable(nodes: Vec<Node>) -> Element {
    rsx! {
        div { class: "overflow-x-auto",
            table { class: "w-full border-collapse text-sm",
                thead {
                    tr { class: "bg-gray-50 border-b border-gray-200 text-left text-xs uppercase tracking-wide text-gray-500",
                        th { class: "px-4 py-2 font-medium", "Host" }
                        th { class: "px-4 py-2 font-medium", "Address" }
                        th { class: "px-4 py-2 font-medium", "Rank" }
                        th { class: "px-4 py-2 font-medium", "Role" }
                        th { class: "px-4 py-2 font-medium", "Status" }
                        th { class: "px-4 py-2 font-medium", "Last seen" }
                    }
                }
                tbody {
                    for node in nodes {
                        {
                            let datetime: DateTime<Utc> = (SystemTime::UNIX_EPOCH
                                + Duration::from_micros(node.timestamp))
                                .into();
                            let timestamp_str = datetime.format("%H:%M:%S").to_string();
                            let url = format!("http://{}", node.addr);
                            let tone = node_status_tone(&node);
                            let (badge_bg, badge_text) = match tone {
                                NodeTone::Healthy => ("bg-emerald-50 text-emerald-700 border-emerald-200", "Healthy"),
                                NodeTone::Degraded => ("bg-red-50 text-red-700 border-red-200", "Issue"),
                                NodeTone::Unknown => ("bg-gray-100 text-gray-600 border-gray-200", "Unknown"),
                            };
                            let status_label = node.status.clone().unwrap_or_else(|| badge_text.to_string());
                            rsx! {
                                tr { class: "border-b border-gray-100 hover:bg-gray-50/80",
                                    td { class: "px-4 py-2.5 font-medium text-gray-900", "{node.host}" }
                                    td { class: "px-4 py-2.5",
                                        a {
                                            href: "{url}",
                                            target: "_blank",
                                            class: format!("text-{} hover:underline font-mono text-xs", colors::PRIMARY),
                                            "{node.addr}"
                                        }
                                    }
                                    td { class: "px-4 py-2.5 font-mono text-gray-700",
                                        {node.rank.map(|r| r.to_string()).unwrap_or_else(|| "—".to_string())}
                                        if let Some(world) = node.world_size {
                                            span { class: "text-gray-400", " / {world}" }
                                        }
                                    }
                                    td { class: "px-4 py-2.5 text-gray-600",
                                        {node.role_name.clone().unwrap_or_else(|| "—".to_string())}
                                    }
                                    td { class: "px-4 py-2.5",
                                        span {
                                            class: "inline-flex px-2 py-0.5 rounded-full text-[11px] font-medium border {badge_bg}",
                                            "{status_label}"
                                        }
                                    }
                                    td { class: "px-4 py-2.5 font-mono text-xs text-gray-500", "{timestamp_str}" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
