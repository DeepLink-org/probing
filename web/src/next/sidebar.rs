//! Focused navigation tree and route-owned controls for the Next UI.

use dioxus::prelude::*;
use dioxus_router::Link;

use crate::api::ApiClient;
use crate::components::icon::Icon;
use crate::components::sidebar::profiling::controls::{
    PprofControls, PyTorchTimelineControls, RayTimelineControls, TorchControls,
    TraceTimelineControls,
};
use crate::state::commands::COMMAND_PANEL_OPEN;
use crate::state::inference::INFERENCE_REFRESH;
use crate::state::llm_config::LLM_SETTINGS_OPEN;
use crate::state::overlays::{open_monitor_overlay, SidebarMonitor};
use crate::state::profiling::{normalize_profiling_view, SPANS_TREE_LIMIT, SPANS_TREE_RELOAD};
use crate::state::rl::{RL_EVENT_LIMIT, ROLLOUT_FILTER, ROLLOUT_FILTER_INPUT};
use crate::state::stack::{
    bump_stack_refresh, stack_tid_label, STACK_DIST_CLUSTER, STACK_DIST_RELOAD, STACK_MODE,
    STACK_SNAPSHOT,
};
use crate::state::training::{TRAINING_CLUSTER_SCOPE, TRAINING_REFRESH};
use crate::state::ui_tasks::ui_tasks_snapshot;
use crate::ui_version::{activate, UiVersion};
use crate::utils::callframe::{mode_for_kind, FrameKind};

use super::routes::NextRoute;
use super::settings::{
    DASHBOARD_AUTO_REFRESH, DASHBOARD_MANUAL_REFRESH, DISTRIBUTED_CLUSTER_SCOPE,
    DISTRIBUTED_REFRESH, DISTRIBUTED_STEP_LIMIT,
};

#[component]
pub(super) fn NextSidebar(
    route: NextRoute,
    #[props(default = false)] compact: bool,
    #[props(optional)] on_navigate: Option<EventHandler<()>>,
    #[props(optional)] on_toggle_compact: Option<EventHandler<()>>,
) -> Element {
    if compact {
        return rsx! {
            CompactSidebar {
                route,
                on_toggle_compact: move |_| {
                    if let Some(handler) = on_toggle_compact {
                        handler.call(());
                    }
                },
            }
        };
    }

    let show_close = on_navigate.is_some();
    let show_compact = on_toggle_compact.is_some();
    let invoke_navigation = move || {
        if let Some(handler) = on_navigate {
            handler.call(());
        }
    };
    let task_count = ui_tasks_snapshot().len();
    let rl_active = is_rl_route(&route);
    let profiles_active = is_profiles_route(&route);
    let stacks_active = is_stacks_route(&route);
    let deep_tools_active = is_deep_tools_route(&route);

    rsx! {
        div { class: "flex h-16 shrink-0 items-center gap-2 border-b border-slate-800 px-3",
            Link {
                to: NextRoute::Dashboard {},
                class: "flex min-w-0 flex-1 items-center gap-2 rounded-md focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-400",
                onclick: move |_| invoke_navigation(),
                img {
                    src: "{crate::utils::base_path::with_base(\"/logo.svg\")}",
                    alt: "Probing",
                    class: "h-8 w-8 shrink-0"
                }
                div { class: "min-w-0",
                    div { class: "truncate text-sm font-semibold", "Probing" }
                    div { class: "truncate text-[10px] uppercase tracking-wider text-blue-300", "Diagnostics workspace" }
                }
            }
            if show_close {
                HeaderButton {
                    label: "Close navigation",
                    icon: &icondata::AiCloseOutlined,
                    onclick: move |_| invoke_navigation(),
                }
            } else if show_compact {
                HeaderButton {
                    label: "Use compact sidebar",
                    icon: &icondata::AiMenuFoldOutlined,
                    onclick: move |_| {
                        if let Some(handler) = on_toggle_compact {
                            handler.call(());
                        }
                    },
                }
            }
        }

        div { class: "shrink-0 px-3 pt-3",
            SearchButton { compact: false }
        }

        nav {
            class: "min-h-0 flex-1 overflow-y-auto overscroll-contain px-3 pb-3 pt-2",
            NavLeaf {
                to: NextRoute::Dashboard {},
                label: "Dashboard",
                icon: &icondata::AiHomeOutlined,
                active: matches!(route, NextRoute::Dashboard {}),
                on_navigate: move |_| invoke_navigation(),
            }
            if matches!(route, NextRoute::Dashboard {}) {
                ControlPanel {
                    title: "Dashboard controls",
                    DashboardControls {}
                }
            }
            NavLeaf {
                to: NextRoute::Investigate {},
                label: "Investigate",
                icon: &icondata::AiRobotOutlined,
                active: matches!(route, NextRoute::Investigate {}),
                on_navigate: move |_| invoke_navigation(),
            }
            if matches!(route, NextRoute::Investigate {}) {
                ControlPanel {
                    title: "Investigation",
                    button {
                        r#type: "button",
                        class: control_button(false),
                        onclick: move |_| *LLM_SETTINGS_OPEN.write() = true,
                        "LLM settings"
                    }
                }
            }
            NavLeaf {
                to: NextRoute::Distributed {},
                label: "Distributed health",
                icon: &icondata::AiClusterOutlined,
                active: matches!(route, NextRoute::Distributed {}),
                on_navigate: move |_| invoke_navigation(),
            }
            if matches!(route, NextRoute::Distributed {}) {
                ControlPanel {
                    title: "Distributed controls",
                    DistributedControls {}
                }
            }
            NavLeaf {
                to: NextRoute::Cluster {},
                label: "Cluster nodes",
                icon: &icondata::AiApartmentOutlined,
                active: matches!(route, NextRoute::Cluster {}),
                on_navigate: move |_| invoke_navigation(),
            }

            NavSection { label: "Workloads" }
            NavLeaf {
                to: NextRoute::Training {},
                label: "Training",
                icon: &icondata::AiRadarChartOutlined,
                active: matches!(route, NextRoute::Training {}),
                on_navigate: move |_| invoke_navigation(),
            }
            if matches!(route, NextRoute::Training {}) {
                ControlPanel {
                    title: "Training controls",
                    TrainingControls {}
                }
            }
            FocusBranch {
                to: NextRoute::Rollout {},
                label: "Reinforcement learning",
                icon: &icondata::AiDeploymentUnitOutlined,
                active: rl_active,
                on_navigate: move |_| invoke_navigation(),
                NavLeaf {
                    to: NextRoute::Rollout {},
                    label: "Rollout",
                    icon: &icondata::AiDeploymentUnitOutlined,
                    active: matches!(route, NextRoute::Rollout {} | NextRoute::RolloutLegacy {}),
                    nested: true,
                    on_navigate: move |_| invoke_navigation(),
                }
                NavLeaf {
                    to: NextRoute::RlTrain {},
                    label: "Policy training",
                    icon: &icondata::AiLineChartOutlined,
                    active: matches!(route, NextRoute::RlTrain {}),
                    nested: true,
                    on_navigate: move |_| invoke_navigation(),
                }
                NavLeaf {
                    to: NextRoute::RlSpans {},
                    label: "Distributed spans",
                    icon: &icondata::AiApartmentOutlined,
                    active: matches!(route, NextRoute::RlSpans {}),
                    nested: true,
                    on_navigate: move |_| invoke_navigation(),
                }
                NavLeaf {
                    to: NextRoute::ProcessTimeline {},
                    label: "Process timeline",
                    icon: &icondata::AiClockCircleOutlined,
                    active: matches!(route, NextRoute::ProcessTimeline {}),
                    nested: true,
                    on_navigate: move |_| invoke_navigation(),
                }
                NavLeaf {
                    to: NextRoute::Perfetto {},
                    label: "Perfetto",
                    icon: &icondata::AiThunderboltOutlined,
                    active: matches!(route, NextRoute::Perfetto {}),
                    nested: true,
                    on_navigate: move |_| invoke_navigation(),
                }
                ControlPanel {
                    title: "RL context",
                    RlControls { route: route.clone() }
                }
            }
            NavLeaf {
                to: NextRoute::Inference {},
                label: "Inference",
                icon: &icondata::AiDashboardOutlined,
                active: matches!(route, NextRoute::Inference {}),
                on_navigate: move |_| invoke_navigation(),
            }
            if matches!(route, NextRoute::Inference {}) {
                ControlPanel {
                    title: "Inference controls",
                    InferenceControls {}
                }
            }

            NavSection { label: "Advanced analysis" }
            FocusBranch {
                to: NextRoute::Profiles {},
                label: "Profiles",
                icon: &icondata::CgPerformance,
                active: profiles_active,
                on_navigate: move |_| invoke_navigation(),
                ProfileNavigation {
                    route: route.clone(),
                    on_navigate: move |_| invoke_navigation(),
                }
                ControlPanel {
                    title: "Capture controls",
                    ProfileControls { route: route.clone() }
                }
            }
            FocusBranch {
                to: NextRoute::Stack {},
                label: "Stacks",
                icon: &icondata::AiApartmentOutlined,
                active: stacks_active,
                on_navigate: move |_| invoke_navigation(),
                StackNavigation {
                    route: route.clone(),
                    on_navigate: move |_| invoke_navigation(),
                }
                ControlPanel {
                    title: "Stack controls",
                    StackControls { route: route.clone() }
                }
            }
            NavLeaf {
                to: NextRoute::Spans {},
                label: "Spans",
                icon: &icondata::AiApiOutlined,
                active: matches!(route, NextRoute::Spans {} | NextRoute::TracesLegacy {}),
                on_navigate: move |_| invoke_navigation(),
            }
            if matches!(route, NextRoute::Spans {} | NextRoute::TracesLegacy {}) {
                ControlPanel {
                    title: "Span controls",
                    SpansControls {}
                }
            }

            NavSection { label: "Deep tools" }
            FocusBranch {
                to: NextRoute::Analytics {},
                label: "Deep tools",
                icon: &icondata::AiToolOutlined,
                active: deep_tools_active,
                on_navigate: move |_| invoke_navigation(),
                NavLeaf {
                    to: NextRoute::Analytics {},
                    label: "SQL Explorer",
                    icon: &icondata::AiDatabaseOutlined,
                    active: matches!(route, NextRoute::Analytics {}),
                    nested: true,
                    on_navigate: move |_| invoke_navigation(),
                }
                NavLeaf {
                    to: NextRoute::Python {},
                    label: "Python Trace",
                    icon: &icondata::SiPython,
                    active: matches!(route, NextRoute::Python {}),
                    nested: true,
                    on_navigate: move |_| invoke_navigation(),
                }
                NavLeaf {
                    to: NextRoute::Pulsing {},
                    label: "Pulsing",
                    icon: &icondata::AiNodeIndexOutlined,
                    active: matches!(route, NextRoute::Pulsing {}),
                    nested: true,
                    on_navigate: move |_| invoke_navigation(),
                }
                NavLeaf {
                    to: NextRoute::System {},
                    label: "Process & system",
                    icon: &icondata::AiControlOutlined,
                    active: matches!(route, NextRoute::System {}),
                    nested: true,
                    on_navigate: move |_| invoke_navigation(),
                }
                NavLeaf {
                    to: NextRoute::Explore {},
                    label: "Capability catalog",
                    icon: &icondata::AiAppstoreOutlined,
                    active: matches!(route, NextRoute::Explore {} | NextRoute::ClassicFallback { .. }),
                    nested: true,
                    on_navigate: move |_| invoke_navigation(),
                }
            }
        }

        SidebarFooter { task_count }
    }
}

#[component]
fn CompactSidebar(route: NextRoute, on_toggle_compact: EventHandler<()>) -> Element {
    let task_count = ui_tasks_snapshot().len();
    rsx! {
        div { class: "flex h-16 shrink-0 items-center justify-center border-b border-slate-800",
            button {
                r#type: "button",
                class: "rounded-lg p-2 hover:bg-slate-900",
                aria_label: "Expand sidebar",
                onclick: move |_| on_toggle_compact.call(()),
                img {
                    src: "{crate::utils::base_path::with_base(\"/logo.svg\")}",
                    alt: "Expand Probing navigation",
                    class: "h-8 w-8"
                }
            }
        }
        div { class: "px-2 pt-3",
            SearchButton { compact: true }
        }
        nav { class: "min-h-0 flex-1 space-y-1 overflow-y-auto px-2 py-3",
            CompactLink { to: NextRoute::Dashboard {}, label: "Dashboard", icon: &icondata::AiHomeOutlined, active: matches!(route, NextRoute::Dashboard {}) }
            CompactLink { to: NextRoute::Investigate {}, label: "Investigate", icon: &icondata::AiRobotOutlined, active: matches!(route, NextRoute::Investigate {}) }
            CompactLink { to: NextRoute::Distributed {}, label: "Distributed health", icon: &icondata::AiClusterOutlined, active: matches!(route, NextRoute::Distributed {}) }
            CompactLink { to: NextRoute::Cluster {}, label: "Cluster nodes", icon: &icondata::AiApartmentOutlined, active: matches!(route, NextRoute::Cluster {}) }
            div { class: "my-2 border-t border-slate-800" }
            CompactLink { to: NextRoute::Training {}, label: "Training", icon: &icondata::AiRadarChartOutlined, active: matches!(route, NextRoute::Training {}) }
            CompactLink { to: NextRoute::Rollout {}, label: "Reinforcement learning", icon: &icondata::AiDeploymentUnitOutlined, active: is_rl_route(&route) }
            CompactLink { to: NextRoute::Inference {}, label: "Inference", icon: &icondata::AiDashboardOutlined, active: matches!(route, NextRoute::Inference {}) }
            div { class: "my-2 border-t border-slate-800" }
            CompactLink { to: NextRoute::Profiles {}, label: "Profiles", icon: &icondata::CgPerformance, active: is_profiles_route(&route) }
            CompactLink { to: NextRoute::Stack {}, label: "Stacks", icon: &icondata::AiApartmentOutlined, active: is_stacks_route(&route) }
            CompactLink { to: NextRoute::Spans {}, label: "Spans", icon: &icondata::AiApiOutlined, active: matches!(route, NextRoute::Spans {} | NextRoute::TracesLegacy {}) }
            CompactLink { to: NextRoute::Analytics {}, label: "Deep tools", icon: &icondata::AiToolOutlined, active: is_deep_tools_route(&route) }
        }
        div { class: "shrink-0 space-y-1 border-t border-slate-800 p-2",
            CompactAction {
                label: format!("Tasks · {task_count}"),
                icon: &icondata::AiUnorderedListOutlined,
                onclick: move |_| open_monitor_overlay(SidebarMonitor::Tasks),
            }
            CompactAction {
                label: "Overhead".to_string(),
                icon: &icondata::AiDashboardOutlined,
                onclick: move |_| open_monitor_overlay(SidebarMonitor::Overhead),
            }
            CompactAction {
                label: "Classic interface".to_string(),
                icon: &icondata::AiSwapOutlined,
                onclick: move |_| activate(UiVersion::Classic),
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
fn SearchButton(compact: bool) -> Element {
    rsx! {
        button {
            r#type: "button",
            class: if compact {
                "flex h-10 w-full items-center justify-center rounded-lg border border-slate-800 bg-slate-900 text-slate-400 hover:border-slate-700 hover:text-white"
            } else {
                "flex w-full items-center gap-2 rounded-lg border border-slate-700 bg-slate-900 px-3 py-2 text-left text-xs text-slate-400 hover:border-slate-600 hover:bg-slate-800 hover:text-slate-200"
            },
            aria_label: "Search capabilities and commands",
            title: if compact { "Search pages and commands · ⌘K" } else { "" },
            onclick: move |_| *COMMAND_PANEL_OPEN.write() = true,
            Icon { icon: &icondata::AiSearchOutlined, class: "h-4 w-4 shrink-0" }
            if !compact {
                span { class: "min-w-0 flex-1 truncate", "Find a page or command" }
                kbd { class: "rounded border border-slate-700 bg-slate-950 px-1.5 py-0.5 text-[9px] text-slate-500", "⌘K" }
            }
        }
    }
}

#[component]
fn NavSection(label: &'static str) -> Element {
    rsx! {
        p { class: "mb-1.5 mt-3 px-2 text-[10px] font-medium uppercase tracking-[0.12em] text-slate-600",
            "{label}"
        }
    }
}

#[component]
fn NavLeaf(
    to: NextRoute,
    label: String,
    icon: &'static icondata::Icon,
    active: bool,
    #[props(default = false)] nested: bool,
    on_navigate: EventHandler<()>,
) -> Element {
    let class = if active {
        "mb-1 flex items-center gap-2 rounded-lg bg-blue-500/20 px-3 py-2 text-sm font-medium text-blue-100"
    } else {
        "mb-1 flex items-center gap-2 rounded-lg px-3 py-2 text-sm font-medium text-slate-300 hover:bg-slate-900 hover:text-white"
    };
    let icon_class = if nested {
        "h-3.5 w-3.5 shrink-0 text-slate-500"
    } else {
        "h-4 w-4 shrink-0"
    };
    rsx! {
        Link {
            to,
            class,
            onclick: move |_| on_navigate.call(()),
            Icon { icon, class: icon_class }
            span { class: "min-w-0 flex-1 truncate", "{label}" }
            if active {
                span { class: "h-1.5 w-1.5 shrink-0 rounded-full bg-blue-300" }
            }
        }
    }
}

#[component]
fn FocusBranch(
    to: NextRoute,
    label: &'static str,
    icon: &'static icondata::Icon,
    active: bool,
    on_navigate: EventHandler<()>,
    children: Element,
) -> Element {
    rsx! {
        Link {
            to,
            class: if active {
                "mb-1 flex items-center gap-2 rounded-lg bg-blue-500/10 px-3 py-2 text-sm font-medium text-blue-100"
            } else {
                "mb-1 flex items-center gap-2 rounded-lg px-3 py-2 text-sm font-medium text-slate-300 hover:bg-slate-900 hover:text-white"
            },
            aria_expanded: active,
            onclick: move |_| on_navigate.call(()),
            Icon { icon, class: "h-4 w-4 shrink-0" }
            span { class: "min-w-0 flex-1 truncate", "{label}" }
            Icon {
                icon: if active { &icondata::AiDownOutlined } else { &icondata::AiRightOutlined },
                class: "h-3.5 w-3.5 shrink-0 text-slate-500"
            }
        }
        if active {
            div { class: "relative mb-1 ml-4 border-l border-slate-800 pl-2",
                {children}
            }
        }
    }
}

#[component]
fn ControlPanel(title: &'static str, children: Element) -> Element {
    rsx! {
        div { class: "mb-2 ml-4 border-l border-blue-500/25 py-1 pl-3",
            div { class: "mb-2 text-[9px] font-semibold uppercase tracking-[0.12em] text-blue-300/70",
                "{title}"
            }
            div { class: "space-y-2", {children} }
        }
    }
}

#[component]
fn DashboardControls() -> Element {
    let enabled = *DASHBOARD_AUTO_REFRESH.read();
    rsx! {
        ToggleRow {
            label: "Auto refresh",
            checked: enabled,
            onchange: move |_| *DASHBOARD_AUTO_REFRESH.write() = !*DASHBOARD_AUTO_REFRESH.read(),
        }
        button {
            r#type: "button",
            class: control_button(true),
            onclick: move |_| *DASHBOARD_MANUAL_REFRESH.write() += 1,
            "Refresh now"
        }
    }
}

#[component]
fn DistributedControls() -> Element {
    let cluster = *DISTRIBUTED_CLUSTER_SCOPE.read();
    let limit = *DISTRIBUTED_STEP_LIMIT.read();
    rsx! {
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
            "Refresh evidence"
        }
    }
}

#[component]
fn TrainingControls() -> Element {
    let cluster = *TRAINING_CLUSTER_SCOPE.read();
    rsx! {
        div { class: "grid grid-cols-2 gap-1.5",
            button {
                r#type: "button",
                class: if !cluster {
                    "rounded-md bg-blue-600 px-2 py-1.5 text-[10px] font-medium text-white"
                } else {
                    "rounded-md border border-slate-700 bg-slate-900 px-2 py-1.5 text-[10px] text-slate-400"
                },
                onclick: move |_| *TRAINING_CLUSTER_SCOPE.write() = false,
                "This node"
            }
            button {
                r#type: "button",
                class: if cluster {
                    "rounded-md bg-blue-600 px-2 py-1.5 text-[10px] font-medium text-white"
                } else {
                    "rounded-md border border-slate-700 bg-slate-900 px-2 py-1.5 text-[10px] text-slate-400"
                },
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
            if cluster { "Scan cluster" } else { "Refresh training" }
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
        if perfetto {
            p { class: "text-[10px] leading-relaxed text-slate-500",
                "Perfetto fetches up to 2000 events per process."
            }
        } else {
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
        button {
            r#type: "button",
            class: control_button(true),
            onclick: move |_| {
                spawn(async move {
                    let _ = ApiClient::new().scrape_inference_engines().await;
                    *INFERENCE_REFRESH.write() += 1;
                });
            },
            "Scrape now"
        }
        p { class: "text-[10px] text-slate-500", "Metrics auto-refresh every 5 seconds." }
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
                nested: true,
                on_navigate: move |_| on_navigate.call(()),
            }
        }
    }
}

#[component]
fn ProfileControls(route: NextRoute) -> Element {
    let view = profile_view(&route);
    let title_class =
        "text-[10px] font-semibold uppercase tracking-wide text-slate-500".to_string();
    let value_class = "text-xs font-mono text-slate-200".to_string();
    let input_class =
        "w-full rounded-md border border-slate-700 bg-slate-950 px-2 py-1.5 text-xs text-slate-200"
            .to_string();
    rsx! {
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
            nested: true,
            on_navigate: move |_| on_navigate.call(()),
        }
        NavLeaf {
            to: NextRoute::DistributedStack {},
            label: "Distributed stack",
            icon: &icondata::AiClusterOutlined,
            active: matches!(route, NextRoute::DistributedStack {}),
            nested: true,
            on_navigate: move |_| on_navigate.call(()),
        }
        NavLeaf {
            to: NextRoute::DistributedPythonStack {},
            label: "Distributed Python",
            icon: &icondata::SiPython,
            active: matches!(route, NextRoute::DistributedPythonStack {}),
            nested: true,
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
            ToggleRow {
                label: "Cluster fan-out",
                checked: cluster,
                onchange: move |_| *STACK_DIST_CLUSTER.write() = !*STACK_DIST_CLUSTER.read(),
            }
            button {
                r#type: "button",
                class: control_button(true),
                onclick: move |_| *STACK_DIST_RELOAD.write() += 1,
                "Reload flamegraph"
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
        div { class: "flex items-center justify-between gap-2 text-[10px] text-slate-500",
            span { "Thread" }
            span { class: "truncate font-mono text-slate-300", "{target}" }
        }
        if snapshot.loaded {
            p { class: "text-[10px] leading-relaxed text-slate-500",
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
            "Refresh stack"
        }
    }
}

#[component]
fn StackModeButton(filter: &'static str, label: &'static str, active: bool) -> Element {
    rsx! {
        button {
            r#type: "button",
            class: if active {
                "rounded-md bg-blue-600 px-2 py-1.5 text-[10px] font-medium text-white"
            } else {
                "rounded-md border border-slate-700 bg-slate-900 px-2 py-1.5 text-[10px] text-slate-400 hover:text-white"
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
            "Refresh spans"
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
                class: "h-4 w-4 rounded border-slate-600 bg-slate-900 text-blue-600",
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
            span { class: "mb-1 flex items-center justify-between gap-2 text-[10px] text-slate-500",
                span { "{label}" }
                span { class: "font-mono text-slate-300", "{value}" }
            }
            input {
                r#type: "range",
                min: "{min}",
                max: "{max}",
                step: "{step}",
                value: "{value}",
                class: "w-full accent-blue-500",
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
fn CompactLink(
    to: NextRoute,
    label: &'static str,
    icon: &'static icondata::Icon,
    active: bool,
) -> Element {
    rsx! {
        Link {
            to,
            class: if active {
                "flex h-10 w-full items-center justify-center rounded-lg bg-blue-500/20 text-blue-200"
            } else {
                "flex h-10 w-full items-center justify-center rounded-lg text-slate-500 hover:bg-slate-900 hover:text-white"
            },
            title: label,
            aria_label: label,
            Icon { icon, class: "h-4 w-4" }
        }
    }
}

#[component]
fn CompactAction(
    label: String,
    icon: &'static icondata::Icon,
    onclick: EventHandler<()>,
) -> Element {
    rsx! {
        button {
            r#type: "button",
            class: "flex h-9 w-full items-center justify-center rounded-lg text-slate-500 hover:bg-slate-900 hover:text-white",
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
        div { class: "shrink-0 border-t border-slate-800 p-3",
            div { class: "grid grid-cols-2 gap-2",
                button {
                    r#type: "button",
                    class: "flex items-center gap-2 rounded-lg border border-slate-800 bg-slate-900 px-2.5 py-2 text-left text-[11px] text-slate-300 hover:border-slate-700 hover:bg-slate-800",
                    onclick: move |_| open_monitor_overlay(SidebarMonitor::Tasks),
                    Icon { icon: &icondata::AiUnorderedListOutlined, class: "h-4 w-4 text-slate-500" }
                    span {
                        span { class: "block text-slate-500", "Tasks" }
                        span { class: "font-medium text-slate-200", "{task_count}" }
                    }
                }
                button {
                    r#type: "button",
                    class: "flex items-center gap-2 rounded-lg border border-slate-800 bg-slate-900 px-2.5 py-2 text-left text-[11px] text-slate-300 hover:border-slate-700 hover:bg-slate-800",
                    onclick: move |_| open_monitor_overlay(SidebarMonitor::Overhead),
                    Icon { icon: &icondata::AiDashboardOutlined, class: "h-4 w-4 text-slate-500" }
                    span {
                        span { class: "block text-slate-500", "Overhead" }
                        span { class: "font-medium text-slate-200", "Inspect" }
                    }
                }
            }
            button {
                r#type: "button",
                class: "mt-2 flex w-full items-center justify-between rounded-md px-2 py-1.5 text-[11px] text-slate-500 hover:bg-slate-900 hover:text-slate-300",
                onclick: move |_| activate(UiVersion::Classic),
                span { "Switch interface" }
                span { class: "flex items-center gap-1 text-slate-400",
                    "Classic"
                    Icon { icon: &icondata::AiSwapOutlined, class: "h-3.5 w-3.5" }
                }
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

fn is_rl_route(route: &NextRoute) -> bool {
    matches!(
        route,
        NextRoute::Rollout {}
            | NextRoute::RolloutLegacy {}
            | NextRoute::RlTrain {}
            | NextRoute::RlSpans {}
            | NextRoute::ProcessTimeline {}
            | NextRoute::Perfetto {}
    )
}

fn is_profiles_route(route: &NextRoute) -> bool {
    matches!(
        route,
        NextRoute::Profiles {}
            | NextRoute::ProfilingLegacy {}
            | NextRoute::ProfileView { .. }
            | NextRoute::ChromeTrace {}
    )
}

fn is_stacks_route(route: &NextRoute) -> bool {
    matches!(
        route,
        NextRoute::Stack {}
            | NextRoute::StackThread { .. }
            | NextRoute::DistributedStack {}
            | NextRoute::DistributedPythonStack {}
    )
}

fn is_deep_tools_route(route: &NextRoute) -> bool {
    matches!(
        route,
        NextRoute::Analytics {}
            | NextRoute::Python {}
            | NextRoute::Pulsing {}
            | NextRoute::System {}
            | NextRoute::Explore {}
            | NextRoute::ClassicFallback { .. }
    )
}

fn control_button(primary: bool) -> &'static str {
    if primary {
        "w-full rounded-md bg-blue-600 px-2.5 py-1.5 text-xs font-medium text-white hover:bg-blue-500"
    } else {
        "w-full rounded-md border border-slate-700 bg-slate-900 px-2.5 py-1.5 text-xs text-slate-300 hover:border-slate-600 hover:text-white"
    }
}

fn control_label() -> &'static str {
    "mb-1 block text-[10px] font-medium uppercase tracking-wide text-slate-500"
}

fn control_input() -> &'static str {
    "w-full rounded-md border border-slate-700 bg-slate-950 px-2 py-1.5 text-xs text-slate-200 placeholder:text-slate-600"
}
