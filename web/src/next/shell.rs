use dioxus::prelude::*;
use dioxus_router::{use_navigator, use_route, Outlet};

use crate::agent::refresh_page_snapshot_for_route;
use crate::api::ApiClient;
use crate::app::Route;
use crate::components::agent::LlmSettingsOverlay;
use crate::components::global_command_panel::{
    CommandBar, FloatingResultToast, GlobalCommandPanel,
};
use crate::components::icon::Icon;
use crate::components::keyboard_shortcuts::{GlobalShortcutInstaller, ShortcutsHelpOverlay};
use crate::components::ui_task_runtime::UiTaskRuntime;
use crate::overhead::OverheadSnapshot;
use crate::state::agent::{AGENT_PANEL_OPEN, AGENT_PANEL_WIDTH};
use crate::state::commands::{FloatingResult, COMMAND_PANEL_OPEN};
use crate::state::investigation::{load_investigation_context, INVESTIGATION_CONTEXT};
use crate::state::investigation_url::InvestigationUrlSync;
use crate::state::llm_config::load_llm_config;
use crate::state::llm_config::LLM_SETTINGS_OPEN;
use crate::state::page_context::{apply_page_descriptor, PAGE_CONTEXT};

use super::pages::InvestigateSession;
use super::routes::NextRoute;
use super::sidebar::NextSidebar as FocusedSidebar;

#[component]
pub fn NextShell() -> Element {
    let route = use_route::<NextRoute>();
    let navigator = use_navigator();
    let mut mobile_menu_open = use_signal(|| false);
    let mut sidebar_compact = use_signal(|| false);
    let mut floating_result = use_signal(|| Option::<FloatingResult>::None);

    let overview = use_resource(|| async move { ApiClient::new().get_overview().await });
    let nodes = use_resource(|| async move { ApiClient::new().get_nodes().await });
    let overhead = use_resource(|| async move { ApiClient::new().fetch_overhead_summary().await });

    use_effect(move || {
        load_investigation_context();
        load_llm_config();
        spawn(async move {
            let _ = ApiClient::new().load_skill_store().await;
        });
    });

    let route_for_context = route.clone();
    use_effect(use_reactive!(|(route_for_context,)| {
        let descriptor = describe_route(&route_for_context);
        apply_page_descriptor(
            descriptor.0.to_string(),
            descriptor.1.to_string(),
            descriptor.2,
            descriptor.3.to_string(),
            descriptor.4,
            INVESTIGATION_CONTEXT.read().summary(),
        );
        if let Some(classic_route) = classic_route_for_snapshot(&route_for_context) {
            spawn(async move {
                refresh_page_snapshot_for_route(classic_route).await;
            });
        }
    }));

    let overview_state = overview.read().clone();
    let nodes_state = nodes.read().clone();
    let overhead_state = overhead.read().clone();
    let target = overview_state
        .as_ref()
        .and_then(|result| result.as_ref().ok())
        .map(|process| {
            process
                .exe
                .rsplit('/')
                .next()
                .filter(|name| !name.is_empty())
                .unwrap_or("training process")
                .to_string()
        })
        .unwrap_or_else(|| "training process".to_string());
    let node_count = nodes_state
        .as_ref()
        .and_then(|result| result.as_ref().ok())
        .map(Vec::len);
    let expected_ranks = nodes_state
        .as_ref()
        .and_then(|result| result.as_ref().ok())
        .and_then(|nodes| nodes.iter().filter_map(|node| node.world_size).max())
        .map(|size| size.max(0) as usize);
    let overhead_pct = overhead_state
        .as_ref()
        .and_then(|result| result.as_ref().ok())
        .and_then(|frame| OverheadSnapshot::from_summary(frame).dispatch_overhead_pct);

    rsx! {
        GlobalShortcutInstaller {}
        UiTaskRuntime {}
        InvestigationUrlSync {}
        if *COMMAND_PANEL_OPEN.read() {
            GlobalCommandPanel {}
        }
        ShortcutsHelpOverlay {}
        LlmSettingsOverlay {}
        FloatingResultToast { result: floating_result }
        div { class: "flex h-screen overflow-hidden bg-gray-50 text-gray-950",
            aside {
                class: if sidebar_compact() {
                    "hidden w-20 shrink-0 border-r border-slate-800 bg-slate-950 text-slate-100 lg:flex lg:flex-col"
                } else {
                    "hidden w-72 shrink-0 border-r border-slate-800 bg-slate-950 text-slate-100 lg:flex lg:flex-col"
                },
                FocusedSidebar {
                    route: route.clone(),
                    compact: sidebar_compact(),
                    on_toggle_compact: move |_| sidebar_compact.set(!sidebar_compact()),
                }
            }

            if mobile_menu_open() {
                div {
                    class: "fixed inset-0 z-50 bg-slate-950/45 lg:hidden",
                    onclick: move |_| mobile_menu_open.set(false),
                }
                aside { class: "fixed inset-y-0 left-0 z-[51] flex w-72 flex-col border-r border-slate-800 bg-slate-950 text-slate-100 shadow-2xl lg:hidden",
                    FocusedSidebar {
                        route: route.clone(),
                        on_navigate: move |_| mobile_menu_open.set(false),
                    }
                }
            }

            div { class: "flex min-w-0 flex-1 flex-col",
                header { class: "shrink-0 border-b border-gray-200 bg-white",
                    div { class: "flex min-h-16 items-center gap-3 px-3 sm:px-5",
                        button {
                            r#type: "button",
                            class: "inline-flex h-10 w-10 shrink-0 items-center justify-center rounded-lg border border-gray-300 text-gray-700 hover:bg-gray-50 lg:hidden",
                            aria_label: "Open navigation",
                            onclick: move |_| mobile_menu_open.set(true),
                            Icon { icon: &icondata::AiMenuOutlined, class: "h-5 w-5" }
                        }
                        div { class: "grid min-w-0 flex-1 grid-cols-2 gap-x-4 gap-y-1 md:grid-cols-4",
                            ContextItem { label: "Target", value: target }
                            ContextItem {
                                label: "Scope",
                                value: match (node_count, expected_ranks) {
                                    (Some(nodes), Some(ranks)) => format!("{nodes} nodes · {ranks} ranks"),
                                    (Some(nodes), None) => format!("{nodes} nodes"),
                                    _ => "This process".to_string(),
                                }
                            }
                            ContextItem { label: "Window", value: "Live · auto refresh".to_string() }
                            ContextItem {
                                label: "Data quality",
                                value: match (node_count, overhead_pct) {
                                    (Some(nodes), Some(pct)) => format!("{nodes} nodes · overhead {pct:+.1}%"),
                                    (Some(nodes), None) => format!("{nodes} nodes · overhead —"),
                                    _ => "Connecting…".to_string(),
                                }
                            }
                        }
                        button {
                            r#type: "button",
                            class: "hidden shrink-0 items-center gap-2 rounded-lg bg-blue-600 px-4 py-2 text-sm font-medium text-white hover:bg-blue-700 sm:inline-flex",
                            onclick: move |_| {
                                navigator.push(NextRoute::Investigate {});
                            },
                            Icon { icon: &icondata::AiRobotOutlined, class: "h-4 w-4" }
                            "Start diagnosis"
                        }
                    }
                }
                div { class: "shrink-0 overflow-x-auto border-b border-gray-200",
                    CommandBar {
                        on_execute_done: move |result| *floating_result.write() = Some(result),
                    }
                }
                div { class: "relative min-h-0 flex-1 overflow-hidden",
                    main { class: "absolute inset-0 overflow-y-auto",
                        div { class: "mx-auto w-full max-w-[1600px] p-4 sm:p-6",
                            Outlet::<NextRoute> {}
                        }
                    }
                    NextAgentPanel {
                        hidden: matches!(route, NextRoute::Investigate {}),
                        route: route.clone(),
                    }
                }
            }
        }
    }
}

#[component]
fn NextAgentPanel(hidden: bool, route: NextRoute) -> Element {
    if hidden || !*AGENT_PANEL_OPEN.read() {
        return rsx! {};
    }

    let width_class = AGENT_PANEL_WIDTH.read().floating_class();
    rsx! {
        div { class: "absolute inset-0 z-40 flex pointer-events-none",
            div {
                class: "flex-1 bg-black/20 pointer-events-auto",
                onclick: move |_| *AGENT_PANEL_OPEN.write() = false,
            }
            aside { class: "flex h-full {width_class} min-w-[22rem] shrink-0 flex-col bg-gray-50 shadow-2xl pointer-events-auto",
                div { class: "flex shrink-0 items-center justify-between border-b border-gray-200 bg-white px-4 py-3",
                    div {
                        div { class: "text-sm font-semibold text-gray-950", "Investigate" }
                        div { class: "text-xs text-gray-500", "Skill-driven diagnostic workspace" }
                    }
                    div { class: "flex items-center gap-1",
                        button {
                            r#type: "button",
                            class: "rounded-md p-2 text-gray-500 hover:bg-gray-100 hover:text-gray-800",
                            aria_label: "Open LLM settings",
                            onclick: move |_| *LLM_SETTINGS_OPEN.write() = true,
                            Icon { icon: &icondata::AiSettingOutlined, class: "h-4 w-4" }
                        }
                        button {
                            r#type: "button",
                            class: "rounded-md p-2 text-gray-500 hover:bg-gray-100 hover:text-gray-800",
                            aria_label: "Close Investigate panel",
                            onclick: move |_| *AGENT_PANEL_OPEN.write() = false,
                            Icon { icon: &icondata::AiCloseOutlined, class: "h-4 w-4" }
                        }
                    }
                }
                div { class: "flex min-h-0 flex-1 flex-col p-3",
                    NextPageEvidence { route }
                    div { class: "min-h-0 flex-1",
                        InvestigateSession { compact: true }
                    }
                }
            }
        }
    }
}

#[component]
fn NextPageEvidence(route: NextRoute) -> Element {
    let page = PAGE_CONTEXT.read().clone();
    rsx! {
        div { class: "mb-3 rounded-xl border border-blue-200 bg-blue-50 px-3 py-2 text-xs text-blue-950",
            div { class: "flex items-start justify-between gap-2",
                div { class: "min-w-0",
                    div { class: "font-semibold", "Viewing · {page.title}" }
                    div { class: "truncate font-mono text-[10px] text-blue-700", "{page.path}" }
                }
                button {
                    r#type: "button",
                    class: "shrink-0 rounded-md border border-blue-200 bg-white px-2 py-1 text-[10px] font-medium text-blue-700 hover:bg-blue-100",
                    disabled: page.snapshot_loading,
                    onclick: move |_| {
                        if let Some(classic_route) = classic_route_for_snapshot(&route) {
                            spawn(async move {
                                refresh_page_snapshot_for_route(classic_route).await;
                            });
                        }
                    },
                    if page.snapshot_loading { "Loading…" } else { "Refresh evidence" }
                }
            }
            if !page.snapshot.is_empty() {
                pre { class: "mt-2 max-h-24 overflow-auto whitespace-pre-wrap rounded-md bg-white/70 p-2 font-mono text-[10px] text-blue-900",
                    "{page.snapshot}"
                }
            }
        }
    }
}

#[component]
fn ContextItem(label: &'static str, value: String) -> Element {
    rsx! {
        div { class: "min-w-0",
            div { class: "text-[10px] font-medium uppercase tracking-wide text-gray-400", "{label}" }
            div { class: "truncate text-xs font-medium text-gray-800 sm:text-sm", "{value}" }
        }
    }
}

fn describe_route(
    route: &NextRoute,
) -> (
    &'static str,
    &'static str,
    String,
    &'static str,
    Vec<String>,
) {
    match route {
        NextRoute::Dashboard {} => (
            "next_dashboard",
            "Training health",
            "/".to_string(),
            "Job progress, rank health, GPU utilization, and prioritized diagnostic findings.",
            vec![
                "health_overview".into(),
                "job_health".into(),
                "slow_rank".into(),
            ],
        ),
        NextRoute::Investigate {} => (
            "next_investigate",
            "Investigate",
            "/agent".to_string(),
            "Skill-driven evidence collection and diagnostic reasoning.",
            vec!["health_overview".into()],
        ),
        NextRoute::Training {} => (
            "next_training",
            "Training",
            "/training".to_string(),
            "Step timing, module hotspots, and collective latency.",
            vec!["slow_rank".into(), "module_bottleneck".into()],
        ),
        NextRoute::Rollout {} | NextRoute::RolloutLegacy {} => (
            "next_rl_rollout",
            "RL Rollout",
            "/rl".to_string(),
            "Per-trajectory phase timing across rollout workers.",
            vec!["health_overview".into(), "module_bottleneck".into()],
        ),
        NextRoute::RlTrain {} => (
            "next_rl_train",
            "RL Train",
            "/rl/train".to_string(),
            "Training batch phases keyed by train step.",
            vec!["slow_rank".into(), "module_bottleneck".into()],
        ),
        NextRoute::RlSpans {} => (
            "next_rl_spans",
            "RL Spans",
            "/rl/spans".to_string(),
            "Distributed RL span hierarchy with cross-process linking.",
            vec!["slow_rank".into(), "comm_bottleneck".into()],
        ),
        NextRoute::ProcessTimeline {} => (
            "next_process_timeline",
            "Process Timeline",
            "/rl/process-timeline".to_string(),
            "Per-process span timing and batch drill-down.",
            vec!["module_bottleneck".into()],
        ),
        NextRoute::Perfetto {} => (
            "next_perfetto",
            "Perfetto",
            "/rl/perfetto".to_string(),
            "Chrome trace export for the loaded RL span set.",
            vec!["module_bottleneck".into(), "comm_bottleneck".into()],
        ),
        NextRoute::Inference {} => (
            "next_inference",
            "Inference",
            "/rl/inference".to_string(),
            "Inference engine throughput, latency, queue, and cache metrics.",
            vec!["gpu_pressure".into()],
        ),
        NextRoute::Distributed {} => (
            "next_distributed",
            "Distributed",
            "/distributed".to_string(),
            "Cluster completeness, rank alignment, and culprit/victim evidence.",
            vec!["slow_rank".into(), "nccl_culprit_victim".into()],
        ),
        NextRoute::Cluster {} => (
            "next_cluster",
            "Cluster",
            "/cluster".to_string(),
            "Registered distributed nodes, roles, ranks, status, and heartbeat age.",
            vec!["job_health".into(), "slow_rank".into()],
        ),
        NextRoute::Stack {}
        | NextRoute::StackThread { .. }
        | NextRoute::DistributedStack {}
        | NextRoute::DistributedPythonStack {} => (
            "next_stacks",
            "Stacks",
            "/stacks".to_string(),
            "Local and distributed Python/native stack evidence.",
            vec!["training_hang".into(), "module_bottleneck".into()],
        ),
        NextRoute::Spans {} | NextRoute::TracesLegacy {} => (
            "next_spans",
            "Spans",
            "/spans".to_string(),
            "Hierarchical spans, filters, attributes, and investigation context.",
            vec!["training_hang".into(), "module_bottleneck".into()],
        ),
        NextRoute::Profiles {}
        | NextRoute::ProfilingLegacy {}
        | NextRoute::ProfileView { .. }
        | NextRoute::ChromeTrace {} => (
            "next_profiles",
            "Profiles",
            "/profiles".to_string(),
            "CPU, Torch, trace, PyTorch, and Ray performance evidence.",
            vec!["module_bottleneck".into(), "comm_bottleneck".into()],
        ),
        NextRoute::Analytics {} => (
            "next_analytics",
            "SQL Explorer",
            "/analytics".to_string(),
            "Local and federated table catalog, SQL editor, previews, and results.",
            vec!["health_overview".into()],
        ),
        NextRoute::Python {} => (
            "next_python",
            "Python Trace",
            "/python".to_string(),
            "Live function variable watches and historical records.",
            vec!["module_bottleneck".into()],
        ),
        NextRoute::Pulsing {} => (
            "next_pulsing",
            "Pulsing",
            "/pulsing".to_string(),
            "Actor system, trace timeline, metrics, and membership.",
            vec!["health_overview".into()],
        ),
        NextRoute::System {} => (
            "next_system",
            "Process & System",
            "/system".to_string(),
            "Detailed CPU/GPU trends, threads, process metadata, and environment.",
            vec!["gpu_pressure".into(), "module_bottleneck".into()],
        ),
        NextRoute::Explore {} | NextRoute::ClassicFallback { .. } => (
            "next_explore",
            "Explore",
            "/explore".to_string(),
            "Advanced classic tools that have not moved into the next shell yet.",
            vec![],
        ),
    }
}

fn classic_route_for_snapshot(route: &NextRoute) -> Option<Route> {
    match route {
        NextRoute::Dashboard {} | NextRoute::System {} => Some(Route::DashboardPage {}),
        NextRoute::Investigate {} => Some(Route::AgentPage {}),
        NextRoute::Training {} => Some(Route::TrainingPage {}),
        NextRoute::Rollout {} | NextRoute::RolloutLegacy {} => Some(Route::RolloutPage {}),
        NextRoute::RlTrain {} => Some(Route::TrainPage {}),
        NextRoute::RlSpans {} => Some(Route::RlSpansPage {}),
        NextRoute::ProcessTimeline {} => Some(Route::ProcessTimelinePage {}),
        NextRoute::Perfetto {} => Some(Route::PerfettoPage {}),
        NextRoute::Inference {} => Some(Route::InferencePage {}),
        NextRoute::Distributed {} | NextRoute::Cluster {} => Some(Route::ClusterPage {}),
        NextRoute::Stack {} => Some(Route::StackPage {}),
        NextRoute::StackThread { tid } => Some(Route::StackWithTidPage { tid: tid.clone() }),
        NextRoute::DistributedStack {} => Some(Route::StackDistributedFullPage {}),
        NextRoute::DistributedPythonStack {} => Some(Route::StackDistributedPyPage {}),
        NextRoute::Spans {} | NextRoute::TracesLegacy {} => Some(Route::SpansPage {}),
        NextRoute::Profiles {} | NextRoute::ProfilingLegacy {} => Some(Route::ProfilingViewPage {
            view: "pprof".to_string(),
        }),
        NextRoute::ProfileView { view } => Some(Route::ProfilingViewPage { view: view.clone() }),
        NextRoute::ChromeTrace {} => Some(Route::ProfilingViewPage {
            view: "trace".to_string(),
        }),
        NextRoute::Analytics {} => Some(Route::AnalyticsPage {}),
        NextRoute::Python {} => Some(Route::PythonPage {}),
        NextRoute::Pulsing {} => Some(Route::PulsingPage {}),
        NextRoute::Explore {} | NextRoute::ClassicFallback { .. } => None,
    }
}
