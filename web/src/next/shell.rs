use dioxus::prelude::*;
use dioxus_router::{use_route, Outlet};

use crate::agent::refresh_page_snapshot_for_route;
use crate::api::ApiClient;
use crate::app::Route;
use crate::components::agent::LlmSettingsOverlay;
use crate::components::global_command_panel::GlobalCommandPanel;
use crate::components::icon::Icon;
use crate::components::keyboard_shortcuts::{GlobalShortcutInstaller, ShortcutsHelpOverlay};
use crate::components::ui_task_runtime::UiTaskRuntime;
use crate::state::agent::{
    load_agent_panel_width, save_agent_panel_width, AgentPanelWidth, AGENT_PANEL_OPEN,
    AGENT_PANEL_WIDTH,
};
use crate::state::commands::COMMAND_PANEL_OPEN;
use crate::state::investigation::{load_investigation_context, INVESTIGATION_CONTEXT};
use crate::state::investigation_url::InvestigationUrlSync;
use crate::state::llm_config::load_llm_config;
use crate::state::llm_config::LLM_SETTINGS_OPEN;
use crate::state::page_context::{apply_page_descriptor, PAGE_CONTEXT};

use super::pages::InvestigateSession;
use super::routes::NextRoute;
use super::settings::{load_next_shell_settings, save_next_sidebar_compact, NEXT_SIDEBAR_COMPACT};
use super::sidebar::NextSidebar as FocusedSidebar;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WorkspaceKind {
    Standard,
    FullHeight,
}

impl WorkspaceKind {
    fn main_class(self) -> &'static str {
        match self {
            Self::Standard => "absolute inset-0 overflow-y-auto",
            Self::FullHeight => "absolute inset-0 overflow-hidden",
        }
    }

    fn content_class(self) -> &'static str {
        match self {
            Self::Standard => "mx-auto w-full max-w-[1600px] p-4 lg:p-5",
            Self::FullHeight => "h-full min-h-0 w-full p-4 lg:p-5",
        }
    }
}

fn workspace_kind(route: &NextRoute) -> WorkspaceKind {
    match route {
        NextRoute::Profiles {}
        | NextRoute::ProfilingLegacy {}
        | NextRoute::ProfileView { .. }
        | NextRoute::ChromeTrace {}
        | NextRoute::Perfetto {} => WorkspaceKind::FullHeight,
        _ => WorkspaceKind::Standard,
    }
}

#[component]
pub fn NextShell() -> Element {
    let route = use_route::<NextRoute>();

    use_effect(move || {
        load_investigation_context();
        load_llm_config();
        load_agent_panel_width();
        load_next_shell_settings();
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

    let sidebar_compact = *NEXT_SIDEBAR_COMPACT.read();
    let sidebar_width = if sidebar_compact { "w-14" } else { "w-72" };
    let workspace = workspace_kind(&route);
    let investigation_route_key = format!("{route:?}");

    rsx! {
        GlobalShortcutInstaller {}
        UiTaskRuntime {}
        InvestigationUrlSync { route_key: investigation_route_key }
        if *COMMAND_PANEL_OPEN.read() {
            GlobalCommandPanel {}
        }
        ShortcutsHelpOverlay {}
        LlmSettingsOverlay {}
        div { class: "flex h-screen overflow-hidden bg-gray-50 text-gray-950",
            a {
                href: "#main-content",
                class: "sr-only fixed left-3 top-3 z-[100] rounded-md bg-white px-3 py-2 text-sm font-semibold text-blue-800 shadow-lg ring-2 ring-blue-600 focus:not-sr-only",
                "Skip to main content"
            }
            aside {
                class: "flex {sidebar_width} shrink-0 flex-col border-r border-slate-800 bg-slate-950 text-slate-100 transition-[width] duration-150",
                FocusedSidebar {
                    route: route.clone(),
                    compact: sidebar_compact,
                    on_toggle_compact: move |_| {
                        save_next_sidebar_compact(!*NEXT_SIDEBAR_COMPACT.read());
                    },
                }
            }

            div { class: "flex min-w-0 flex-1 flex-col",
                div { class: "relative min-h-0 flex-1 overflow-hidden",
                    main { id: "main-content", tabindex: "-1", class: "{workspace.main_class()}",
                        div { class: "{workspace.content_class()}",
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

    let panel_width = *AGENT_PANEL_WIDTH.read();
    let width_class = panel_width.floating_class();
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
                        div { class: "mr-1 inline-flex items-center rounded-md border border-gray-200 bg-gray-100 p-0.5",
                            button {
                                r#type: "button",
                                class: if panel_width == AgentPanelWidth::Third {
                                    "rounded bg-white px-2 py-1 text-xs font-medium text-gray-900 shadow-sm"
                                } else {
                                    "rounded px-2 py-1 text-xs text-gray-500 hover:text-gray-800"
                                },
                                aria_label: "Use one-third Investigate width",
                                onclick: move |_| save_agent_panel_width(AgentPanelWidth::Third),
                                "⅓"
                            }
                            button {
                                r#type: "button",
                                class: if panel_width == AgentPanelWidth::TwoThirds {
                                    "rounded bg-white px-2 py-1 text-xs font-medium text-gray-900 shadow-sm"
                                } else {
                                    "rounded px-2 py-1 text-xs text-gray-500 hover:text-gray-800"
                                },
                                aria_label: "Use two-thirds Investigate width",
                                onclick: move |_| save_agent_panel_width(AgentPanelWidth::TwoThirds),
                                "⅔"
                            }
                        }
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
                    div { class: "truncate font-mono text-xs text-blue-700", "{page.path}" }
                }
                button {
                    r#type: "button",
                    class: "shrink-0 rounded-md border border-blue-200 bg-white px-2 py-1 text-xs font-medium text-blue-700 hover:bg-blue-100",
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
                pre { class: "mt-2 max-h-24 overflow-auto whitespace-pre-wrap rounded-md bg-white/70 p-2 font-mono text-xs text-blue-900",
                    "{page.snapshot}"
                }
            }
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
        NextRoute::DistributedStatus {} => (
            "next_distributed_status",
            "Distributed Status",
            "/cluster/status".to_string(),
            "PyTorch wait counters and read-only rendezvous store state.",
            vec!["training_hang".into(), "comm_bottleneck".into()],
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
        NextRoute::Distributed {} | NextRoute::Cluster {} | NextRoute::DistributedStatus {} => {
            Some(Route::ClusterPage {})
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visualization_routes_receive_full_height_workspace() {
        assert_eq!(
            workspace_kind(&NextRoute::ProfileView {
                view: "trace".to_string(),
            }),
            WorkspaceKind::FullHeight
        );
        assert_eq!(
            workspace_kind(&NextRoute::Perfetto {}),
            WorkspaceKind::FullHeight
        );
        assert_eq!(
            workspace_kind(&NextRoute::Training {}),
            WorkspaceKind::Standard
        );
    }
}
