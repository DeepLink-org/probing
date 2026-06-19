//! Sidebar: logo, nav list, Profiling submenu, footer.
//! Uses [colors](crate::components::colors). Width/visibility in [state::sidebar](crate::state::sidebar).

use dioxus::prelude::*;
use dioxus_router::{use_route, Link};

use crate::app::Route;
use crate::components::colors::colors;
use crate::components::icon::Icon;
use crate::state::sidebar::{
    load_sidebar_state, save_sidebar_state, SIDEBAR_HIDDEN, SIDEBAR_WIDTH,
};

mod nav_item;
mod profiling;
mod resize;
mod stack;
mod tasks;

use nav_item::{SidebarNavItem, SidebarSectionLabel};
use profiling::ProfilingSidebarItem;
use resize::ResizeHandle;
use stack::StackSidebarItem;
use tasks::SidebarTaskQueue;

fn sidebar_classes() -> (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
) {
    (
        colors::SIDEBAR_ASIDE,
        colors::SIDEBAR_LOGO_BORDER,
        colors::SIDEBAR_BRAND,
        colors::SIDEBAR_FOOTER,
        colors::SIDEBAR_FOOTER_LINK,
        colors::SIDEBAR_HIDE_BTN,
    )
}

#[component]
pub fn Sidebar() -> Element {
    let route = use_route::<Route>();
    let mut show_profiling_dropdown = use_signal(|| false);
    let mut show_stack_dropdown = use_signal(|| false);

    use_effect(move || {
        load_sidebar_state();
    });

    let route_for_profiling = route.clone();
    use_effect(move || {
        if matches!(route_for_profiling, Route::ProfilingViewPage { .. }) {
            *show_profiling_dropdown.write() = true;
        }
    });

    let route_for_stack = route.clone();
    use_effect(move || {
        if matches!(
            route_for_stack,
            Route::StackPage {} | Route::StackWithTidPage { .. }
        ) {
            *show_stack_dropdown.write() = true;
        }
    });

    let width = *SIDEBAR_WIDTH.read();
    let (aside, logo_border, brand, footer, footer_link, hide_btn) = sidebar_classes();
    let main_style = format!("width: {}px;", width);

    rsx! {
        div {
            class: "relative flex h-screen",
            style: "{main_style}",
            aside {
                class: "{aside}",
                style: "{main_style}",
                div {
                    class: "{logo_border}",
                    Link {
                        to: Route::DashboardPage {},
                        class: "flex items-center gap-2",
                        img { src: "{crate::utils::base_path::with_base(\"/assets/logo.svg\")}", alt: "Probing", class: "w-7 h-7 flex-shrink-0" }
                        span { class: "{brand}", "Probing" }
                    }
                }

                nav {
                    class: "flex-1 overflow-y-auto py-3",
                    div { class: "px-2 space-y-0.5",
                        SidebarSectionLabel { label: "Overview" }
                        SidebarNavItem {
                            to: Route::DashboardPage {},
                            icon: &icondata::AiLineChartOutlined,
                            label: "Dashboard",
                            is_active: route == Route::DashboardPage {},
                        }
                        SidebarNavItem {
                            to: Route::AgentPage {},
                            icon: &icondata::AiRobotOutlined,
                            label: "Investigate",
                            title: "Playbook-driven investigation (diagnostic agent)",
                            is_active: route == Route::AgentPage {},
                        }
                        StackSidebarItem {
                            show_dropdown: show_stack_dropdown,
                        }

                        SidebarSectionLabel { label: "Analysis" }
                        ProfilingSidebarItem {
                            show_dropdown: show_profiling_dropdown,
                        }
                        SidebarNavItem {
                            to: Route::AnalyticsPage {},
                            icon: &icondata::AiAreaChartOutlined,
                            label: "Analytics",
                            is_active: route == Route::AnalyticsPage {},
                        }
                        SidebarNavItem {
                            to: Route::SpansPage {},
                            icon: &icondata::AiApiOutlined,
                            label: "Spans",
                            title: "Hierarchical tracing spans from python.trace_event",
                            is_active: matches!(route, Route::TracesPage {} | Route::SpansPage {}),
                        }
                        SidebarNavItem {
                            to: Route::TrainingPage {},
                            icon: &icondata::AiRadarChartOutlined,
                            label: "Training",
                            is_active: route == Route::TrainingPage {},
                        }
                        SidebarNavItem {
                            to: Route::PulsingPage {},
                            icon: &icondata::AiDeploymentUnitOutlined,
                            label: "Pulsing",
                            is_active: route == Route::PulsingPage {},
                        }

                        SidebarSectionLabel { label: "System" }
                        SidebarNavItem {
                            to: Route::ClusterPage {},
                            icon: &icondata::AiClusterOutlined,
                            label: "Cluster",
                            is_active: route == Route::ClusterPage {},
                        }
                        SidebarNavItem {
                            to: Route::PythonPage {},
                            icon: &icondata::SiPython,
                            label: "Python",
                            title: "Live variable tracing on functions (not distributed spans)",
                            is_active: route == Route::PythonPage {},
                        }
                    }
                }

                SidebarTaskQueue {}

                div { class: "{footer}",
                    a {
                        href: "https://github.com/reiase/probing",
                        target: "_blank",
                        class: "{footer_link}",
                        Icon { icon: &icondata::AiGithubOutlined, class: "w-4 h-4" }
                        span { "GitHub" }
                    }
                }
            }

            button {
                class: "{hide_btn} focus:outline-none focus:ring-2 focus:ring-blue-400 focus:ring-offset-2 focus:ring-offset-slate-900",
                title: "Hide Sidebar",
                aria_label: "Hide sidebar",
                onclick: move |_| {
                    *SIDEBAR_HIDDEN.write() = true;
                    save_sidebar_state();
                },
                Icon {
                    icon: &icondata::AiMenuFoldOutlined,
                    class: "w-4 h-4 text-slate-300"
                }
            }

            ResizeHandle {}
        }
    }
}
