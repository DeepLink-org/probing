//! Focused navigation tree and route-owned controls for the Next UI.

use dioxus::prelude::*;

use crate::api::ApiClient;
use crate::components::icon::Icon;
use crate::components::profiling_controls::{
    PprofControls, PyTorchTimelineControls, RayTimelineControls, TorchControls,
    TraceTimelineControls,
};
use crate::state::commands::COMMAND_PANEL_OPEN;
use crate::state::inference::INFERENCE_REFRESH;
use crate::state::investigation::INVESTIGATION_CONTEXT;
use crate::state::llm_config::LLM_SETTINGS_OPEN;
use crate::state::overlays::{open_monitor_overlay, SidebarMonitor};
use crate::state::profiling::{
    normalize_profiling_view, PROFILING_PPROF_FREQ, SPANS_TREE_LIMIT, SPANS_TREE_RELOAD,
};
use crate::state::rl::{RL_EVENT_LIMIT, ROLLOUT_FILTER, ROLLOUT_FILTER_INPUT};
use crate::state::stack::{
    bump_stack_refresh, stack_tid_label, STACK_DIST_CLUSTER, STACK_DIST_RELOAD, STACK_MODE,
    STACK_SNAPSHOT,
};
use crate::state::training::{
    PlacementAvailability, TRAINING_CLUSTER_SCOPE, TRAINING_PLACEMENT_AVAILABILITY,
    TRAINING_REFRESH,
};
use crate::state::ui_tasks::running_ui_task_count;
use crate::utils::callframe::{mode_for_kind, FrameKind};

use super::components::evidence_href;
use super::page_registry::SidebarTool;
use super::routes::NextRoute;
use super::settings::{
    DASHBOARD_AUTO_REFRESH, DASHBOARD_MANUAL_REFRESH, DISTRIBUTED_CLUSTER_SCOPE,
    DISTRIBUTED_REFRESH, DISTRIBUTED_STEP_LIMIT, MEMORY_CLUSTER_SCOPE, MEMORY_REFRESH,
    MEMORY_WINDOW_MINUTES,
};

#[component]
pub(super) fn NextSidebar(
    route: NextRoute,
    #[props(default = false)] compact: bool,
    #[props(optional)] on_navigate: Option<EventHandler<()>>,
    #[props(optional)] on_toggle_compact: Option<EventHandler<()>>,
) -> Element {
    let invoke_navigation = move || {
        if let Some(handler) = on_navigate {
            handler.call(());
        }
    };
    let task_count = running_ui_task_count();

    rsx! {
        div { class: "flex h-full min-h-0 w-full",
            SidebarRail {
                route: route.clone(),
                compact,
                task_count,
                on_navigate: move |_| invoke_navigation(),
                on_toggle_compact: move |_| {
                    if let Some(handler) = on_toggle_compact {
                        handler.call(());
                    }
                },
            }
            if !compact {
                div { class: "flex min-w-0 flex-1 flex-col border-l border-slate-800 bg-slate-950",
                    SidebarDetailHeader {
                        route: route.clone(),
                        on_toggle_compact: move |_| {
                            if let Some(handler) = on_toggle_compact {
                                handler.call(());
                            }
                        },
                    }
                    div { class: "min-h-0 flex-1 overflow-y-auto overscroll-contain p-3",
                        ActiveSidebarPanel {
                            route: route.clone(),
                            on_navigate: move |_| invoke_navigation(),
                        }
                    }
                    SidebarFooter { task_count }
                }
            }
        }
    }
}

#[component]
fn SidebarRail(
    route: NextRoute,
    compact: bool,
    task_count: usize,
    on_navigate: EventHandler<()>,
    on_toggle_compact: EventHandler<()>,
) -> Element {
    let dashboard_href = evidence_href(
        &NextRoute::Dashboard {},
        &INVESTIGATION_CONTEXT.read().clone(),
    );
    rsx! {
        div { class: "flex w-14 shrink-0 flex-col bg-slate-950",
            div { class: "flex h-16 shrink-0 items-center justify-center border-b border-slate-800",
                a {
                    href: dashboard_href,
                    class: "rounded-lg p-1.5 focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-400",
                    aria_label: "Probing dashboard",
                    onclick: move |_| on_navigate.call(()),
                    img {
                        src: "{crate::utils::base_path::with_base(\"/logo.svg\")}",
                        alt: "Probing",
                        class: "h-8 w-8"
                    }
                }
            }
            div { class: "shrink-0 px-2 pt-2",
                RailAction {
                    label: "Search pages and commands · ⌘K".to_string(),
                    icon: &icondata::AiSearchOutlined,
                    onclick: move |_| *COMMAND_PANEL_OPEN.write() = true,
                }
            }
            nav { class: "min-h-0 flex-1 space-y-1 overflow-y-auto px-2 py-2",
                RailLink { to: NextRoute::Dashboard {}, label: "Dashboard", icon: &icondata::AiHomeOutlined, active: route.page_spec().sidebar_tool == SidebarTool::Dashboard, on_navigate }
                RailLink { to: NextRoute::Investigate {}, label: "Investigate", icon: &icondata::AiRobotOutlined, active: route.page_spec().sidebar_tool == SidebarTool::Investigate, on_navigate }
                RailLink { to: NextRoute::Distributed {}, label: "Cluster", icon: &icondata::AiClusterOutlined, active: route.page_spec().sidebar_tool == SidebarTool::Cluster, on_navigate }
                RailSeparator {}
                RailLink { to: NextRoute::Training {}, label: "Training", icon: &icondata::AiRadarChartOutlined, active: route.page_spec().sidebar_tool == SidebarTool::Training, on_navigate }
                RailLink { to: NextRoute::Inference {}, label: "Inference", icon: &icondata::AiDashboardOutlined, active: route.page_spec().sidebar_tool == SidebarTool::Inference, on_navigate }
                RailLink { to: NextRoute::Rollout {}, label: "RL", icon: &icondata::AiDeploymentUnitOutlined, active: route.page_spec().sidebar_tool == SidebarTool::Rl, on_navigate }
                RailSeparator {}
                RailLink { to: NextRoute::Memory {}, label: "Memory", icon: &icondata::AiDatabaseOutlined, active: route.page_spec().sidebar_tool == SidebarTool::Memory, on_navigate }
                RailLink { to: NextRoute::Profiles {}, label: "Profiling", icon: &icondata::CgPerformance, active: route.page_spec().sidebar_tool == SidebarTool::Profiling, on_navigate }
                RailLink { to: NextRoute::Stack {}, label: "Stacks", icon: &icondata::AiApartmentOutlined, active: route.page_spec().sidebar_tool == SidebarTool::Stacks, on_navigate }
                RailLink { to: NextRoute::Spans {}, label: "Tracing", icon: &icondata::AiApiOutlined, active: route.page_spec().sidebar_tool == SidebarTool::Tracing, on_navigate }
                RailSeparator {}
                RailLink { to: NextRoute::Analytics {}, label: "Deep tools", icon: &icondata::AiToolOutlined, active: route.page_spec().sidebar_tool == SidebarTool::DeepTools, on_navigate }
            }
            if compact {
                div { class: "shrink-0 space-y-1 border-t border-slate-800 p-2",
                    RailAction { label: format!("Tasks · {task_count}"), icon: &icondata::AiUnorderedListOutlined, onclick: move |_| open_monitor_overlay(SidebarMonitor::Tasks) }
                    RailAction { label: "Overhead".to_string(), icon: &icondata::AiDashboardOutlined, onclick: move |_| open_monitor_overlay(SidebarMonitor::Overhead) }
                    RailAction { label: "Expand sidebar".to_string(), icon: &icondata::AiMenuUnfoldOutlined, onclick: move |_| on_toggle_compact.call(()) }
                }
            }
        }
    }
}

#[component]
fn HeaderButton(
    label: &'static str,
    icon: &'static icondata::Icon,
    onclick: EventHandler<()>,
) -> Element {
    rsx! {
        button {
            r#type: "button",
            class: "rounded-md p-2 text-slate-400 hover:bg-slate-800 hover:text-white focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-400",
            aria_label: label,
            title: label,
            onclick: move |_| onclick.call(()),
            Icon { icon, class: "h-4 w-4" }
        }
    }
}

#[component]
fn SidebarDetailHeader(route: NextRoute, on_toggle_compact: EventHandler<()>) -> Element {
    let spec = route.page_spec();
    let group = spec.sidebar_group;
    let title = spec.sidebar_title;
    rsx! {
        div { class: "flex h-16 shrink-0 items-center justify-between gap-2 border-b border-slate-800 px-3",
            div { class: "min-w-0",
                div { class: "truncate text-xs font-semibold uppercase tracking-[0.14em] text-slate-400", "{group}" }
                div { class: "truncate text-sm font-semibold text-slate-100", "{title}" }
            }
            HeaderButton {
                label: "Collapse sidebar",
                icon: &icondata::AiMenuFoldOutlined,
                onclick: move |_| on_toggle_compact.call(()),
            }
        }
    }
}

#[component]
fn ActiveSidebarPanel(route: NextRoute, on_navigate: EventHandler<()>) -> Element {
    rsx! {
        match route.clone() {
            NextRoute::Dashboard {} => rsx! {
                SidebarIntro { text: "Cluster step health and local accelerator evidence." }
                ControlPanel { title: "Dashboard controls", DashboardControls {} }
            },
            NextRoute::Investigate {} => rsx! {
                SidebarIntro { text: "Run evidence-driven diagnostic skills." }
                ControlPanel { title: "Investigation", button { r#type: "button", class: control_button(false), onclick: move |_| *LLM_SETTINGS_OPEN.write() = true, "LLM settings" } }
            },
            NextRoute::Distributed {} | NextRoute::Cluster {} | NextRoute::DistributedStatus {} => rsx! {
                SidebarSectionLabel { label: "Views" }
                NavLeaf { to: NextRoute::Distributed {}, label: "Overview", icon: &icondata::AiClusterOutlined, active: matches!(route, NextRoute::Distributed {}), on_navigate }
                NavLeaf { to: NextRoute::Cluster {}, label: "Nodes", icon: &icondata::AiApartmentOutlined, active: matches!(route, NextRoute::Cluster {}), on_navigate }
                NavLeaf { to: NextRoute::DistributedStatus {}, label: "Distributed Status", icon: &icondata::AiDeploymentUnitOutlined, active: matches!(route, NextRoute::DistributedStatus {}), on_navigate }
                if matches!(route, NextRoute::Distributed {}) {
                    ControlPanel { title: "Cluster controls", DistributedControls {} }
                } else {
                    ControlPanel { title: "Cluster controls", ClusterRefreshControl {} }
                }
            },
            NextRoute::Training {} => rsx! {
                SidebarIntro { text: "Step trend, placement, compute, and communication evidence." }
                ControlPanel { title: "Training controls", TrainingControls {} }
            },
            NextRoute::Inference {} => rsx! {
                SidebarIntro { text: "Throughput, latency, queue, and cache evidence." }
                ControlPanel { title: "Inference controls", InferenceControls {} }
            },
            NextRoute::Rollout {} | NextRoute::RolloutLegacy {} | NextRoute::RlTrain {} | NextRoute::RlSpans {} | NextRoute::ProcessTimeline {} | NextRoute::Perfetto {} => rsx! {
                SidebarSectionLabel { label: "Views" }
                NavLeaf { to: NextRoute::Rollout {}, label: "Rollout", icon: &icondata::AiDeploymentUnitOutlined, active: matches!(route, NextRoute::Rollout {} | NextRoute::RolloutLegacy {}), on_navigate }
                NavLeaf { to: NextRoute::RlTrain {}, label: "Policy training", icon: &icondata::AiLineChartOutlined, active: matches!(route, NextRoute::RlTrain {}), on_navigate }
                NavLeaf { to: NextRoute::RlSpans {}, label: "Distributed spans", icon: &icondata::AiApartmentOutlined, active: matches!(route, NextRoute::RlSpans {}), on_navigate }
                NavLeaf { to: NextRoute::ProcessTimeline {}, label: "Process timeline", icon: &icondata::AiClockCircleOutlined, active: matches!(route, NextRoute::ProcessTimeline {}), on_navigate }
                NavLeaf { to: NextRoute::Perfetto {}, label: "Perfetto", icon: &icondata::AiThunderboltOutlined, active: matches!(route, NextRoute::Perfetto {}), on_navigate }
                ControlPanel { title: "RL context", RlControls { route: route.clone() } }
            },
            NextRoute::Profiles {} | NextRoute::ProfilingLegacy {} | NextRoute::ProfileView { .. } | NextRoute::ChromeTrace {} => rsx! {
                SidebarSectionLabel { label: "Views" }
                ProfileNavigation { route: route.clone(), on_navigate }
                ControlPanel { title: "Capture controls", ProfileControls { route: route.clone() } }
            },
            NextRoute::Stack {} | NextRoute::StackThread { .. } | NextRoute::DistributedStack {} | NextRoute::DistributedPythonStack {} => rsx! {
                SidebarSectionLabel { label: "Views" }
                StackNavigation { route: route.clone(), on_navigate }
                ControlPanel { title: "Stack controls", StackControls { route: route.clone() } }
            },
            NextRoute::Spans {} | NextRoute::TracesLegacy {} => rsx! {
                SidebarIntro { text: "Expand the span hierarchy to the required diagnostic depth." }
                ControlPanel { title: "Tracing controls", SpansControls {} }
            },
            NextRoute::Memory {} => rsx! {
                SidebarIntro { text: "Device capacity, sampled peaks, allocator state, and allocation evidence." }
                ControlPanel { title: "Memory controls", MemoryControls {} }
            },
            NextRoute::Analytics {} | NextRoute::Python {} | NextRoute::Pulsing {} | NextRoute::System {} | NextRoute::Explore {} | NextRoute::ClassicFallback { .. } => rsx! {
                SidebarSectionLabel { label: "Tools" }
                NavLeaf { to: NextRoute::Analytics {}, label: "SQL Explorer", icon: &icondata::AiDatabaseOutlined, active: matches!(route, NextRoute::Analytics {}), on_navigate }
                NavLeaf { to: NextRoute::Python {}, label: "Python Trace", icon: &icondata::SiPython, active: matches!(route, NextRoute::Python {}), on_navigate }
                NavLeaf { to: NextRoute::Pulsing {}, label: "Pulsing", icon: &icondata::AiNodeIndexOutlined, active: matches!(route, NextRoute::Pulsing {}), on_navigate }
                NavLeaf { to: NextRoute::System {}, label: "Process snapshot", icon: &icondata::AiControlOutlined, active: matches!(route, NextRoute::System {}), on_navigate }
                NavLeaf { to: NextRoute::Explore {}, label: "Capability catalog", icon: &icondata::AiAppstoreOutlined, active: matches!(route, NextRoute::Explore {} | NextRoute::ClassicFallback { .. }), on_navigate }
            },
        }
    }
}

#[component]
fn NavLeaf(
    to: NextRoute,
    label: String,
    icon: &'static icondata::Icon,
    active: bool,
    on_navigate: EventHandler<()>,
) -> Element {
    let href = evidence_href(&to, &INVESTIGATION_CONTEXT.read().clone());
    let class = if active {
        "mb-1 flex items-center gap-2 rounded-lg bg-blue-500/20 px-3 py-2 text-sm font-medium text-blue-100 focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-400"
    } else {
        "mb-1 flex items-center gap-2 rounded-lg px-3 py-2 text-sm font-medium text-slate-300 hover:bg-slate-900 hover:text-white focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-400"
    };
    rsx! {
        a {
            href,
            class,
            aria_current: if active { "page" } else { "false" },
            onclick: move |_| on_navigate.call(()),
            Icon { icon, class: "h-4 w-4 shrink-0 text-slate-400" }
            span { class: "min-w-0 flex-1 truncate", "{label}" }
            if active {
                span { class: "h-1.5 w-1.5 shrink-0 rounded-full bg-blue-300" }
            }
        }
    }
}

#[component]
fn SidebarIntro(text: &'static str) -> Element {
    rsx! {
        p { class: "mb-4 text-xs leading-relaxed text-slate-400", "{text}" }
    }
}

#[component]
fn SidebarSectionLabel(label: &'static str) -> Element {
    rsx! {
        div { class: "mb-2 px-2 text-xs font-semibold uppercase tracking-[0.14em] text-slate-400", "{label}" }
    }
}

#[component]
fn ControlPanel(title: &'static str, children: Element) -> Element {
    rsx! {
        section { class: "mt-4 rounded-lg border border-slate-800 bg-slate-900/55 p-3",
            div { class: "mb-3 text-xs font-semibold uppercase tracking-[0.14em] text-blue-300", "{title}" }
            div { class: "space-y-2.5", {children} }
        }
    }
}

#[component]
fn RailSeparator() -> Element {
    rsx! {
        div { class: "my-2 border-t border-slate-800" }
    }
}

#[component]
fn DashboardControls() -> Element {
    let enabled = *DASHBOARD_AUTO_REFRESH.read();
    rsx! {
        ControlSummary {
            scope: "Cluster steps + local GPU".to_string(),
            update: if enabled { "Every 5s".to_string() } else { "Manual".to_string() },
        }
        ToggleRow {
            label: "Auto refresh",
            checked: enabled,
            onchange: move |_| *DASHBOARD_AUTO_REFRESH.write() = !*DASHBOARD_AUTO_REFRESH.read(),
        }
        button {
            r#type: "button",
            class: control_button(true),
            onclick: move |_| *DASHBOARD_MANUAL_REFRESH.write() += 1,
            "Refresh dashboard"
        }
    }
}

#[component]
fn DistributedControls() -> Element {
    let cluster = *DISTRIBUTED_CLUSTER_SCOPE.read();
    let limit = *DISTRIBUTED_STEP_LIMIT.read();
    rsx! {
        ControlSummary {
            scope: if cluster { "Cluster fan-out".to_string() } else { "Current process".to_string() },
            update: format!("{limit} steps · changes apply immediately"),
        }
        ToggleRow {
            label: "Cluster fan-out",
            checked: cluster,
            onchange: move |_| {
                *DISTRIBUTED_CLUSTER_SCOPE.write() = !*DISTRIBUTED_CLUSTER_SCOPE.read();
            },
        }
        RangeControl {
            label: "Step samples",
            value: limit,
            min: 64,
            max: 1024,
            step: 64,
            oninput: move |value| *DISTRIBUTED_STEP_LIMIT.write() = value,
        }
        button {
            r#type: "button",
            class: control_button(true),
            onclick: move |_| *DISTRIBUTED_REFRESH.write() += 1,
            if cluster { "Refresh cluster evidence" } else { "Refresh local evidence" }
        }
    }
}

#[component]
fn MemoryControls() -> Element {
    let cluster = *MEMORY_CLUSTER_SCOPE.read();
    let window = *MEMORY_WINDOW_MINUTES.read();
    rsx! {
        ControlSummary {
            scope: if cluster { "Cluster fan-out".to_string() } else { "Current process".to_string() },
            update: format!("{window}m window · every 5s"),
        }
        ToggleRow {
            label: "Cluster fan-out",
            checked: cluster,
            onchange: move |_| *MEMORY_CLUSTER_SCOPE.write() = !*MEMORY_CLUSTER_SCOPE.read(),
        }
        RangeControl {
            label: "Window (minutes)",
            value: window,
            min: 1,
            max: 30,
            step: 1,
            oninput: move |value| *MEMORY_WINDOW_MINUTES.write() = value,
        }
        button {
            r#type: "button",
            class: control_button(true),
            onclick: move |_| *MEMORY_REFRESH.write() += 1,
            if cluster { "Refresh cluster memory" } else { "Refresh local memory" }
        }
    }
}

#[component]
fn ClusterRefreshControl() -> Element {
    rsx! {
        ControlSummary {
            scope: "Current cluster registry".to_string(),
            update: "Explicit refresh".to_string(),
        }
        button {
            r#type: "button",
            class: control_button(true),
            onclick: move |_| *DISTRIBUTED_REFRESH.write() += 1,
            "Refresh cluster evidence"
        }
    }
}

#[component]
fn TrainingControls() -> Element {
    let cluster = *TRAINING_CLUSTER_SCOPE.read();
    let placement = *TRAINING_PLACEMENT_AVAILABILITY.read();
    rsx! {
        ControlSummary {
            scope: if cluster { "Cluster fan-out".to_string() } else { "Current process".to_string() },
            update: "Every 5s · scope applies immediately".to_string(),
        }
        div { class: "grid grid-cols-2 gap-1.5",
            button {
                r#type: "button",
                class: if !cluster {
                    "rounded-md bg-blue-600 px-2 py-1.5 text-xs font-medium text-white focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-400"
                } else {
                    "rounded-md border border-slate-700 bg-slate-900 px-2 py-1.5 text-xs text-slate-300 focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-400"
                },
                aria_pressed: (!cluster).to_string(),
                onclick: move |_| *TRAINING_CLUSTER_SCOPE.write() = false,
                "This node"
            }
            button {
                r#type: "button",
                class: if cluster {
                    "rounded-md bg-blue-600 px-2 py-1.5 text-xs font-medium text-white focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-400"
                } else {
                    "rounded-md border border-slate-700 bg-slate-900 px-2 py-1.5 text-xs text-slate-300 focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-400"
                },
                aria_pressed: cluster.to_string(),
                onclick: move |_| {
                    *TRAINING_CLUSTER_SCOPE.write() = true;
                    *TRAINING_REFRESH.write() += 1;
                },
                "Cluster"
            }
        }
        button {
            r#type: "button",
            class: control_button(true),
            onclick: move |_| *TRAINING_REFRESH.write() += 1,
            if cluster { "Refresh cluster training" } else { "Refresh local training" }
        }
        if placement == PlacementAvailability::Missing {
            PlacementNotice {
                detail: "No distributed heartbeat or rank topology has been reported."
            }
        } else if placement == PlacementAvailability::RegistryUnavailable {
            PlacementNotice {
                detail: "The node heartbeat registry could not be read. Refresh to retry."
            }
        }
    }
}

#[component]
fn PlacementNotice(detail: &'static str) -> Element {
    rsx! {
        div {
            class: "rounded-md border border-amber-400/25 bg-amber-400/[0.06] px-2.5 py-2",
            div { class: "flex items-center gap-1.5 text-xs font-semibold text-amber-200",
                Icon { icon: &icondata::AiWarningOutlined, class: "h-3.5 w-3.5 shrink-0" }
                "Placement unavailable"
            }
            p { class: "mt-1 text-xs leading-relaxed text-slate-400", "{detail}" }
        }
    }
}

#[component]
fn RlControls(route: NextRoute) -> Element {
    let navigator = dioxus_router::use_navigator();
    let limit = *RL_EVENT_LIMIT.read();
    let rollout_input = ROLLOUT_FILTER_INPUT.read().clone();
    let show_rollout = matches!(route, NextRoute::Rollout {} | NextRoute::RolloutLegacy {});
    let perfetto = matches!(route, NextRoute::Perfetto {});
    rsx! {
        ControlSummary {
            scope: if show_rollout { "Selected rollout".to_string() } else { "Current process events".to_string() },
            update: if show_rollout {
                "Load applies the typed rollout ID".to_string()
            } else if perfetto {
                "Up to 2000 events per process".to_string()
            } else {
                format!("Up to {limit} events · limit applies immediately")
            },
        }
        if show_rollout {
            label { class: "block",
                span { class: control_label(), "Rollout ID" }
                input {
                    r#type: "text",
                    value: "{rollout_input}",
                    placeholder: "Latest",
                    class: control_input(),
                    oninput: move |event| *ROLLOUT_FILTER_INPUT.write() = event.value(),
                }
            }
            div { class: "grid grid-cols-2 gap-2",
                button {
                    r#type: "button",
                    class: control_button(true),
                    onclick: move |_| {
                        *ROLLOUT_FILTER.write() = ROLLOUT_FILTER_INPUT.read().trim().to_string();
                        navigator.push(NextRoute::Rollout {});
                    },
                    "Load"
                }
                button {
                    r#type: "button",
                    class: control_button(false),
                    onclick: move |_| {
                        *ROLLOUT_FILTER_INPUT.write() = String::new();
                        *ROLLOUT_FILTER.write() = String::new();
                    },
                    "Clear"
                }
            }
        }
        if !perfetto {
            RangeControl {
                label: "Event limit",
                value: limit,
                min: 100,
                max: 2000,
                step: 100,
                oninput: move |value| *RL_EVENT_LIMIT.write() = value,
            }
        }
    }
}

#[component]
fn InferenceControls() -> Element {
    rsx! {
        ControlSummary {
            scope: "Registered engine endpoints".to_string(),
            update: "Scrape writes a new metric sample".to_string(),
        }
        button {
            r#type: "button",
            class: control_button(true),
            onclick: move |_| {
                spawn(async move {
                    let _ = ApiClient::new().scrape_inference_engines().await;
                    *INFERENCE_REFRESH.write() += 1;
                });
            },
            "Scrape registered engines"
        }
    }
}

#[component]
fn ProfileNavigation(route: NextRoute, on_navigate: EventHandler<()>) -> Element {
    let active = profile_view(&route);
    rsx! {
        for (id, label, icon) in [
            ("pprof", "CPU pprof", &icondata::CgPerformance),
            ("torch", "Torch modules", &icondata::AiFireOutlined),
            ("trace", "Chrome trace", &icondata::AiFundProjectionScreenOutlined),
            ("pytorch", "PyTorch profiler", &icondata::AiLineChartOutlined),
            ("ray", "Ray timeline", &icondata::AiNodeIndexOutlined),
        ] {
            NavLeaf {
                to: NextRoute::ProfileView { view: id.to_string() },
                label,
                icon,
                active: active == id,
                on_navigate: move |_| on_navigate.call(()),
            }
        }
    }
}

#[component]
fn ProfileControls(route: NextRoute) -> Element {
    let view = profile_view(&route);
    let (scope, update) = match view {
        "pprof" => {
            let frequency = *PROFILING_PPROF_FREQ.read();
            (
                "Current process CPU samples".to_string(),
                if frequency <= 0 {
                    "Disabled · choose a non-zero frequency to enable".to_string()
                } else {
                    format!("{frequency} Hz · frequency applies immediately")
                },
            )
        }
        "torch" => (
            "Current process module hooks".to_string(),
            "Toggle applies immediately".to_string(),
        ),
        "trace" => (
            "Current process trace buffer".to_string(),
            "Limit applies immediately · Reload reads the buffer".to_string(),
        ),
        "pytorch" => (
            "Current process PyTorch profiler".to_string(),
            "Steps apply to the next explicit capture".to_string(),
        ),
        "ray" => (
            "Current process Ray timeline".to_string(),
            "Capture occurs on explicit reload".to_string(),
        ),
        _ => ("Current process".to_string(), "Explicit action".to_string()),
    };
    let title_class = "text-xs font-semibold uppercase tracking-wide text-slate-400".to_string();
    let value_class = "text-xs font-mono text-slate-200".to_string();
    let input_class =
        "w-full rounded-md border border-slate-700 bg-slate-950 px-2 py-1.5 text-xs text-slate-200"
            .to_string();
    rsx! {
        ControlSummary { scope, update }
        match view {
            "pprof" => rsx! {
                PprofControls {
                    control_title_class: title_class,
                    control_value_class: value_class,
                }
            },
            "torch" => rsx! {
                TorchControls {
                    control_title_class: title_class,
                    toggle_enabled_class: "relative inline-flex h-5 w-10 rounded-full bg-blue-600".to_string(),
                    toggle_disabled_class: "relative inline-flex h-5 w-10 rounded-full bg-slate-700".to_string(),
                    toggle_label_class: "text-xs text-slate-300".to_string(),
                }
            },
            "trace" => rsx! {
                TraceTimelineControls {
                    control_title_class: title_class,
                    control_value_class: value_class,
                    input_class,
                }
            },
            "pytorch" => rsx! {
                PyTorchTimelineControls { control_title_class: title_class, input_class }
            },
            "ray" => rsx! {
                RayTimelineControls { control_title_class: title_class }
            },
            _ => rsx! {},
        }
    }
}

#[component]
fn StackNavigation(route: NextRoute, on_navigate: EventHandler<()>) -> Element {
    rsx! {
        NavLeaf {
            to: NextRoute::Stack {},
            label: "Local stack",
            icon: &icondata::AiCodeOutlined,
            active: matches!(route, NextRoute::Stack {} | NextRoute::StackThread { .. }),
            on_navigate: move |_| on_navigate.call(()),
        }
        NavLeaf {
            to: NextRoute::DistributedStack {},
            label: "Distributed stack",
            icon: &icondata::AiClusterOutlined,
            active: matches!(route, NextRoute::DistributedStack {}),
            on_navigate: move |_| on_navigate.call(()),
        }
        NavLeaf {
            to: NextRoute::DistributedPythonStack {},
            label: "Distributed Python",
            icon: &icondata::SiPython,
            active: matches!(route, NextRoute::DistributedPythonStack {}),
            on_navigate: move |_| on_navigate.call(()),
        }
    }
}

#[component]
fn StackControls(route: NextRoute) -> Element {
    if matches!(
        route,
        NextRoute::DistributedStack {} | NextRoute::DistributedPythonStack {}
    ) {
        let cluster = *STACK_DIST_CLUSTER.read();
        return rsx! {
            ControlSummary {
                scope: if cluster { "Cluster fan-out".to_string() } else { "Current process".to_string() },
                update: "Capture on explicit reload".to_string(),
            }
            ToggleRow {
                label: "Cluster fan-out",
                checked: cluster,
                onchange: move |_| *STACK_DIST_CLUSTER.write() = !*STACK_DIST_CLUSTER.read(),
            }
            button {
                r#type: "button",
                class: control_button(true),
                onclick: move |_| *STACK_DIST_RELOAD.write() += 1,
                "Capture distributed stacks"
            }
        };
    }

    let tid = match &route {
        NextRoute::StackThread { tid } => Some(tid.as_str()),
        _ => None,
    };
    let target = stack_tid_label(tid);
    let snapshot = STACK_SNAPSHOT.read().clone();
    let mode = STACK_MODE();
    rsx! {
        ControlSummary {
            scope: format!("Thread {target}"),
            update: format!("{mode} frames · capture on refresh"),
        }
        div { class: "flex items-center justify-between gap-2 text-xs text-slate-400",
            span { "Thread" }
            span { class: "truncate font-mono text-slate-300", "{target}" }
        }
        if snapshot.loaded {
            p { class: "text-xs leading-relaxed text-slate-400",
                "{snapshot.shown}/{snapshot.total} frames · py {snapshot.py} · rust {snapshot.rust} · native {snapshot.cpp}"
            }
        }
        div { class: "grid grid-cols-2 gap-1.5",
            StackModeButton { filter: "mixed", label: "All", active: mode == "mixed" }
            StackModeButton { filter: mode_for_kind(FrameKind::Python), label: "Python", active: mode == mode_for_kind(FrameKind::Python) }
            StackModeButton { filter: mode_for_kind(FrameKind::Rust), label: "Rust", active: mode == mode_for_kind(FrameKind::Rust) }
            StackModeButton { filter: mode_for_kind(FrameKind::Cpp), label: "Native", active: mode == mode_for_kind(FrameKind::Cpp) }
        }
        button {
            r#type: "button",
            class: control_button(true),
            onclick: move |_| bump_stack_refresh(),
            "Capture stack now"
        }
    }
}

#[component]
fn StackModeButton(filter: &'static str, label: &'static str, active: bool) -> Element {
    rsx! {
        button {
            r#type: "button",
            class: if active {
                "rounded-md bg-blue-600 px-2 py-1.5 text-xs font-medium text-white"
            } else {
                "rounded-md border border-slate-700 bg-slate-900 px-2 py-1.5 text-xs text-slate-400 hover:text-white"
            },
            onclick: move |_| *STACK_MODE.write() = filter.to_string(),
            "{label}"
        }
    }
}

#[component]
fn SpansControls() -> Element {
    let limit = *SPANS_TREE_LIMIT.read();
    rsx! {
        ControlSummary {
            scope: "Current process trace buffer".to_string(),
            update: format!("Up to {limit} rows · limit applies immediately"),
        }
        RangeControl {
            label: "Tree rows",
            value: limit,
            min: 100,
            max: 5000,
            step: 100,
            oninput: move |value| *SPANS_TREE_LIMIT.write() = value,
        }
        button {
            r#type: "button",
            class: control_button(true),
            onclick: move |_| *SPANS_TREE_RELOAD.write() += 1,
            "Reload span evidence"
        }
    }
}

#[component]
fn ControlSummary(scope: String, update: String) -> Element {
    rsx! {
        dl { class: "grid grid-cols-[3.5rem_minmax(0,1fr)] gap-x-2 gap-y-1 border-b border-slate-800 pb-2 text-xs",
            dt { class: "text-slate-500", "Scope" }
            dd { class: "min-w-0 break-words text-slate-300", "{scope}" }
            dt { class: "text-slate-500", "Update" }
            dd { class: "min-w-0 break-words text-slate-300", "{update}" }
        }
    }
}

#[component]
fn ToggleRow(label: &'static str, checked: bool, onchange: EventHandler<()>) -> Element {
    rsx! {
        label { class: "flex cursor-pointer items-center justify-between gap-3 text-xs text-slate-300",
            span { "{label}" }
            input {
                r#type: "checkbox",
                checked,
                class: "h-5 w-5 rounded border-slate-600 bg-slate-900 text-blue-600 focus:ring-2 focus:ring-blue-400 focus:ring-offset-1 focus:ring-offset-slate-900",
                onchange: move |_| onchange.call(()),
            }
        }
    }
}

#[component]
fn RangeControl(
    label: &'static str,
    value: usize,
    min: usize,
    max: usize,
    step: usize,
    oninput: EventHandler<usize>,
) -> Element {
    rsx! {
        label { class: "block",
            span { class: "mb-1 flex items-center justify-between gap-2 text-xs text-slate-400",
                span { "{label}" }
                span { class: "font-mono text-slate-300", "{value}" }
            }
            input {
                r#type: "range",
                min: "{min}",
                max: "{max}",
                step: "{step}",
                value: "{value}",
                class: "w-full accent-blue-500 focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-400",
                aria_label: label,
                oninput: move |event| {
                    if let Ok(value) = event.value().parse::<usize>() {
                        oninput.call(value);
                    }
                },
            }
        }
    }
}

#[component]
fn RailLink(
    to: NextRoute,
    label: &'static str,
    icon: &'static icondata::Icon,
    active: bool,
    on_navigate: EventHandler<()>,
) -> Element {
    let href = evidence_href(&to, &INVESTIGATION_CONTEXT.read().clone());
    rsx! {
        a {
            href,
            class: if active {
                "flex h-10 w-full items-center justify-center rounded-lg bg-blue-500/20 text-blue-200 focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-400"
            } else {
                "flex h-10 w-full items-center justify-center rounded-lg text-slate-400 hover:bg-slate-900 hover:text-white focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-400"
            },
            title: label,
            aria_label: label,
            aria_current: if active { "page" } else { "false" },
            onclick: move |_| on_navigate.call(()),
            Icon { icon, class: "h-4 w-4" }
        }
    }
}

#[component]
fn RailAction(label: String, icon: &'static icondata::Icon, onclick: EventHandler<()>) -> Element {
    rsx! {
        button {
            r#type: "button",
            class: "flex h-9 w-full items-center justify-center rounded-lg text-slate-400 hover:bg-slate-900 hover:text-white focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-400",
            title: "{label}",
            aria_label: "{label}",
            onclick: move |_| onclick.call(()),
            Icon { icon, class: "h-4 w-4" }
        }
    }
}

#[component]
fn SidebarFooter(task_count: usize) -> Element {
    rsx! {
        div { class: "flex h-11 shrink-0 items-center gap-1 border-t border-slate-800 px-2",
            button {
                r#type: "button",
                class: "flex min-w-0 flex-1 items-center gap-1.5 rounded-md px-2 py-1.5 text-xs text-slate-400 hover:bg-slate-900 hover:text-slate-200 focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-400",
                onclick: move |_| open_monitor_overlay(SidebarMonitor::Tasks),
                title: "Open tasks",
                aria_label: "Open tasks",
                Icon { icon: &icondata::AiUnorderedListOutlined, class: "h-3.5 w-3.5" }
                span { class: "truncate", "Tasks · {task_count}" }
            }
            button {
                r#type: "button",
                class: "rounded-md p-1.5 text-slate-400 hover:bg-slate-900 hover:text-slate-200 focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-400",
                title: "Inspect overhead",
                aria_label: "Inspect overhead",
                onclick: move |_| open_monitor_overlay(SidebarMonitor::Overhead),
                Icon { icon: &icondata::AiDashboardOutlined, class: "h-3.5 w-3.5" }
            }
        }
    }
}

fn profile_view(route: &NextRoute) -> &'static str {
    match route {
        NextRoute::ProfileView { view } => normalize_profiling_view(view),
        NextRoute::ChromeTrace {} => "trace",
        _ => "pprof",
    }
}

fn control_button(primary: bool) -> &'static str {
    if primary {
        "w-full rounded-md bg-blue-600 px-2.5 py-1.5 text-xs font-medium text-white hover:bg-blue-500 focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-400"
    } else {
        "w-full rounded-md border border-slate-700 bg-slate-900 px-2.5 py-1.5 text-xs text-slate-300 hover:border-slate-600 hover:text-white focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-400"
    }
}

fn control_label() -> &'static str {
    "mb-1 block text-xs font-medium uppercase tracking-wide text-slate-400"
}

fn control_input() -> &'static str {
    "w-full rounded-md border border-slate-700 bg-slate-950 px-2 py-1.5 text-xs text-slate-200 placeholder:text-slate-400 focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-400"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_registry_groups_routes_by_stable_tool() {
        assert_eq!(
            (
                NextRoute::Training {}.page_spec().sidebar_group,
                NextRoute::Training {}.page_spec().sidebar_title,
            ),
            ("Workloads", "Training"),
        );
        assert_eq!(
            {
                let spec = NextRoute::ProfileView {
                    view: "trace".to_string(),
                }
                .page_spec();
                (spec.sidebar_group, spec.sidebar_title)
            },
            ("Advanced analysis", "Profiling")
        );
        assert_eq!(
            {
                let spec = NextRoute::DistributedPythonStack {}.page_spec();
                (spec.sidebar_group, spec.sidebar_title)
            },
            ("Advanced analysis", "Stacks")
        );
        assert_eq!(
            {
                let spec = NextRoute::Memory {}.page_spec();
                (spec.sidebar_group, spec.sidebar_title)
            },
            ("Advanced analysis", "Memory")
        );
        assert_eq!(
            {
                let spec = NextRoute::System {}.page_spec();
                (spec.sidebar_group, spec.sidebar_title)
            },
            ("Deep tools", "Toolbox")
        );
        assert_eq!(
            {
                let spec = NextRoute::DistributedStatus {}.page_spec();
                (spec.sidebar_group, spec.sidebar_title)
            },
            ("Workspace", "Cluster")
        );
    }
}
