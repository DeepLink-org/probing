use dioxus::prelude::*;
use dioxus_router::Link;
use probing_proto::prelude::CallFrame;

use crate::api::ApiClient;
use crate::app::Route;
use crate::components::callstack_view::CallStackView;
use crate::components::common::{AsyncBoundary, EmptyState, ErrorState};
use crate::components::flamegraph::{FlamegraphPayload, FlamegraphView};
use crate::components::page::{PageContainer, PageTitle};
use crate::components::profiling::{ProfilingContentPanel, ProfilingErrorPanel};
use crate::hooks::use_app_resource;
use crate::state::stack::{
    stack_tid_label, StackSnapshot, STACK_DIST_CLUSTER, STACK_DIST_RELOAD, STACK_MODE,
    STACK_REFRESH, STACK_SNAPSHOT,
};
use crate::utils::callframe::{count_by_kind, matches_mode};
use crate::utils::error::AppError;

#[component]
pub fn Stack(tid: Option<String>) -> Element {
    let tid_for_api = tid.clone();
    let tid_label = stack_tid_label(tid.as_deref());
    let refresh_tick = STACK_REFRESH();

    rsx! {
        PageContainer {
            PageTitle {
                title: "Stacks".to_string(),
                subtitle: if tid.is_some() {
                    Some(format!("Thread {tid_label}"))
                } else {
                    None
                },
                icon: Some(&icondata::AiApartmentOutlined),
            }

            AsyncBoundary {
                message: Some("Loading call stack…".to_string()),
                StackLoaded {
                    tid: tid_for_api,
                    tid_label: tid_label,
                    refresh_tick: refresh_tick,
                }
            }
        }
    }
}

#[component]
fn StackLoaded(tid: Option<String>, tid_label: String, refresh_tick: u32) -> Element {
    let mode = STACK_MODE();
    let filter_mode = mode.clone();
    let stack = use_app_resource(move || {
        let _ = refresh_tick;
        let tid_arg = tid.clone();
        async move {
            ApiClient::new()
                .get_callstack_with_mode(tid_arg, "mixed")
                .await
        }
    });

    let stack_peek = stack.read().clone();
    let tid_for_effect = tid_label.clone();

    use_effect(use_reactive!(|(
        mode,
        refresh_tick,
        stack_peek,
        tid_for_effect,
    )| {
        let _ = refresh_tick;
        let Some(result) = stack_peek.as_ref() else {
            return;
        };
        *STACK_SNAPSHOT.write() = stack_snapshot_for(&tid_for_effect, result, &mode);
    }));

    match stack.suspend()?().as_ref() {
        Err(err) => rsx! {
            ErrorState {
                title: Some("Failed to load stack".to_string()),
                error: err.display_message(),
            }
        },
        Ok(callframes) if callframes.is_empty() => rsx! {
            EmptyState {
                message: format!(
                    "No stack frames for thread {tid_label}. The thread may be idle or not yet sampled."
                )
            }
        },
        Ok(callframes) => {
            let current_mode = filter_mode.clone();
            let filtered: Vec<_> = callframes
                .iter()
                .filter(|cf| matches_mode(cf, current_mode.as_str()))
                .cloned()
                .collect();
            let shown = filtered.len();

            if filtered.is_empty() {
                rsx! {
                    EmptyState {
                        message: format!(
                            "No frames match the \"{}\" filter",
                            mode_label(&current_mode)
                        )
                    }
                }
            } else {
                rsx! {
                    div { class: "space-y-0",
                        for (idx, cf) in filtered.iter().enumerate() {
                            CallStackView {
                                key: "{refresh_tick}-{idx}",
                                callstack: cf.clone(),
                                index: idx,
                                is_last: idx + 1 == shown,
                                default_open: idx == 0,
                            }
                        }
                    }
                }
            }
        }
    }
}

fn stack_snapshot_for(
    tid_label: &str,
    result: &Result<Vec<CallFrame>, AppError>,
    mode: &str,
) -> StackSnapshot {
    match result {
        Err(_) => StackSnapshot::default(),
        Ok(frames) if frames.is_empty() => StackSnapshot {
            tid_label: tid_label.to_string(),
            loaded: true,
            ..StackSnapshot::default()
        },
        Ok(frames) => {
            let (py_count, rust_count, cpp_count) = count_by_kind(frames);
            let shown = frames.iter().filter(|cf| matches_mode(cf, mode)).count();
            StackSnapshot {
                tid_label: tid_label.to_string(),
                total: frames.len(),
                py: py_count,
                rust: rust_count,
                cpp: cpp_count,
                shown,
                loaded: true,
            }
        }
    }
}

fn mode_label(mode: &str) -> &'static str {
    match mode {
        "py" => "Python",
        "rust" => "Rust",
        "cpp" => "Native",
        _ => "All",
    }
}

/// Distributed stack flamegraph — merge identical stacks across ranks.
#[component]
pub fn StackDistributed(mode: String) -> Element {
    let reload = *STACK_DIST_RELOAD.read();
    let cluster = *STACK_DIST_CLUSTER.read();
    let api_mode = if mode == "py" { "py" } else { "mixed" };
    let on_full = api_mode == "mixed";
    let subtitle = if api_mode == "py" {
        "Distributed flamegraph · Python frames merged across ranks".to_string()
    } else {
        "Distributed flamegraph · full mixed stack merged across ranks".to_string()
    };

    rsx! {
        PageContainer {
            PageTitle {
                title: "Distributed flamegraph".to_string(),
                subtitle: Some(subtitle),
                icon: Some(&icondata::AiClusterOutlined),
            }
            div { class: "flex flex-col flex-1 min-h-0 min-w-0 gap-3",
                div {
                    class: "flex gap-1 border-b border-slate-700/80",
                    DistViewTab {
                        label: "Full stack",
                        active: on_full,
                        route: Route::StackDistributedFullPage {},
                    }
                    DistViewTab {
                        label: "Python only",
                        active: !on_full,
                        route: Route::StackDistributedPyPage {},
                    }
                }
                ProfilingContentPanel {
                    AsyncBoundary {
                        message: Some("Loading distributed flamegraph…".to_string()),
                        StackDistributedFlamegraph {
                            key: "stack-dist-{reload}-{cluster}-{api_mode}",
                            cluster,
                            mode: api_mode.to_string(),
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn DistViewTab(label: &'static str, active: bool, route: Route) -> Element {
    let class = if active {
        "px-3 py-2 text-sm font-medium border-b-2 border-blue-500 text-blue-200 -mb-px"
    } else {
        "px-3 py-2 text-sm text-slate-400 hover:text-slate-200 border-b-2 border-transparent -mb-px"
    };
    rsx! {
        Link {
            to: route,
            class: "{class}",
            "{label}"
        }
    }
}

#[component]
fn StackDistributedFlamegraph(cluster: bool, mode: String) -> Element {
    let payload = use_app_resource(move || {
        let mode = mode.clone();
        async move {
            let body = ApiClient::new()
                .get_distributed_stack_flamegraph_json(cluster, &mode)
                .await?;
            let parsed: FlamegraphPayload = serde_json::from_str(&body)
                .map_err(|e| AppError::Api(format!("Invalid flamegraph JSON: {e}")))?;
            Ok(parsed)
        }
    });

    match payload.suspend()?() {
        Ok(data) => rsx! {
            FlamegraphView {
                payload: data,
                thread_tid: None,
                torch_metric: None,
                on_torch_metric: None,
            }
        },
        Err(err) => rsx! {
            ProfilingErrorPanel {
                title: "Distributed stack flamegraph".to_string(),
                error: err.display_message(),
            }
        },
    }
}
