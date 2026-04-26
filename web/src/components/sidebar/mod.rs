//! Sidebar: logo, nav list, Profiling submenu, footer.
//! Uses [colors](crate::components::colors). Width/visibility in [state::sidebar](crate::state::sidebar).

use dioxus::prelude::*;
use dioxus_router::{Link, use_route};

use crate::app::Route;
use crate::components::colors::colors;
use crate::components::icon::Icon;
use crate::state::sidebar::{load_sidebar_state, save_sidebar_state, SIDEBAR_HIDDEN, SIDEBAR_WIDTH};

mod nav_item;
mod profiling;
mod resize;

use nav_item::SidebarNavItem;
use profiling::ProfilingSidebarItem;
use resize::ResizeHandle;

fn sidebar_classes() -> (String, String, String, String, String, String) {
    (
        format!(
            "bg-gradient-to-b from-{} via-{} to-{} border-r border-{} h-screen flex flex-col flex-shrink-0 shadow-xl",
            colors::SIDEBAR_BG,
            colors::SIDEBAR_BG_VIA,
            colors::SIDEBAR_BG,
            colors::SIDEBAR_BORDER
        ),
        format!("px-4 py-3 border-b border-{}", colors::SIDEBAR_BORDER),
        format!("text-base font-semibold text-{}", colors::SIDEBAR_TEXT_PRIMARY),
        format!("px-4 py-3 border-t border-{}", colors::SIDEBAR_BORDER),
        format!(
            "flex items-center gap-2 text-xs text-{} hover:text-{} transition-colors",
            colors::SIDEBAR_TEXT_MUTED,
            colors::PRIMARY_TEXT_DARK
        ),
        format!(
            "absolute top-4 -right-3 w-6 h-6 bg-{} border border-slate-700 rounded-full shadow-lg flex items-center justify-center hover:bg-slate-600 z-30 transition-colors",
            colors::SIDEBAR_ACTIVE_BG
        ),
    )
}

#[component]
pub fn Sidebar() -> Element {
    let route = use_route::<Route>();
    let show_profiling_dropdown = use_signal(|| false);

    use_effect(move || {
        load_sidebar_state();
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
                        img { src: "/assets/logo.svg", alt: "Probing", class: "w-7 h-7 flex-shrink-0" }
                        span { class: "{brand}", "Probing" }
                    }
                }

                nav {
                    class: "flex-1 overflow-y-auto py-3",
                    div { class: "px-2 space-y-0.5",
                        SidebarNavItem {
                            to: Route::DashboardPage {},
                            icon: &icondata::AiLineChartOutlined,
                            label: "Dashboard",
                            is_active: route == Route::DashboardPage {},
                        }
                        SidebarNavItem {
                            to: Route::StackPage {},
                            icon: &icondata::AiThunderboltOutlined,
                            label: "Stacks",
                            is_active: route == Route::StackPage {},
                        }
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
                            to: Route::TracesPage {},
                            icon: &icondata::AiApiOutlined,
                            label: "Traces",
                            is_active: route == Route::TracesPage {},
                        }
                        SidebarNavItem {
                            to: Route::PulsingPage {},
                            icon: &icondata::AiDeploymentUnitOutlined,
                            label: "Pulsing",
                            is_active: route == Route::PulsingPage {},
                        }
                        div { class: "pt-2" }
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
                            is_active: route == Route::PythonPage {},
                        }
                    }
                }

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
