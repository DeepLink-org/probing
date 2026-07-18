//! Advanced / low-frequency navigation items.

use dioxus::prelude::*;
use dioxus_router::{Link, use_route};
use icondata::Icon as IconData;

use crate::app::Route;
use crate::components::sidebar::nav_item::sidebar_item_class;
use crate::components::icon::Icon;

#[component]
pub fn AdvancedSidebarItem(show_dropdown: Signal<bool>) -> Element {
    let route = use_route::<Route>();
    let is_active = matches!(
        route,
        Route::DashboardPage {}
            | Route::ClusterPage {}
            | Route::PythonPage {}
            | Route::ChromeTracingRedirect {}
    );
    let expanded = *show_dropdown.read();
    let button_class = format!(
        "w-full focus:outline-none focus:ring-2 focus:ring-blue-400 focus:ring-offset-2 focus:ring-offset-slate-900 {}",
        sidebar_item_class(is_active)
    );

    rsx! {
        div {
            button {
                class: "{button_class}",
                aria_expanded: if expanded { "true" } else { "false" },
                aria_label: "Advanced menu",
                onclick: move |_| {
                    let current = *show_dropdown.read();
                    *show_dropdown.write() = !current;
                },
                Icon { icon: &icondata::AiSettingOutlined, class: "w-4 h-4" }
                span { "Advanced" }
            }

            if expanded {
                div {
                    class: "ml-4 mt-0.5 space-y-0.5",
                    AdvancedSubLink {
                        to: Route::DashboardPage {},
                        label: "Dashboard",
                        icon: &icondata::AiLineChartOutlined,
                    }
                    AdvancedSubLink {
                        to: Route::ClusterPage {},
                        label: "Cluster",
                        icon: &icondata::AiClusterOutlined,
                    }
                    AdvancedSubLink {
                        to: Route::PythonPage {},
                        label: "Python",
                        icon: &icondata::SiPython,
                    }
                    AdvancedSubLink {
                        to: Route::ChromeTracingRedirect {},
                        label: "Chrome Tracing",
                        icon: &icondata::AiThunderboltOutlined,
                    }
                }
            }
        }
    }
}

#[component]
fn AdvancedSubLink(to: Route, label: &'static str, icon: &'static IconData) -> Element {
    let route = use_route::<Route>();
    let is_selected = route == to;
    let link_class = format!("{}", sidebar_item_class(is_selected));

    rsx! {
        Link {
            to: to,
            class: "{link_class}",
            Icon { icon, class: "w-4 h-4" }
            span { "{label}" }
        }
    }
}
