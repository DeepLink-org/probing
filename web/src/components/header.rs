use dioxus::prelude::*;
use dioxus_router::{Link, use_route};
use icondata::Icon as IconData;

use crate::app::Route;
use crate::components::icon::Icon;

#[component]
pub fn Header() -> Element {
    rsx! {
        header {
            class: "bg-white shadow-sm border-b border-gray-200",
            div {
                class: "px-6 py-4",
                div {
                    class: "flex items-center justify-between",
                    // Logo and Brand
                    div {
                        class: "flex items-center space-x-4",
                                Link {
                                    to: Route::DashboardPage {},
                                    class: "text-xl font-bold text-gray-900 hover:text-blue-600",
                                    "Probing Dashboard"
                                }
                    }
                    
                    // Top Navigation Tabs
                    nav {
                        class: "hidden md:flex items-center space-x-1",
                                NavTab {
                                    to: Route::DashboardPage {},
                                    icon: &icondata::AiLineChartOutlined,
                                    label: "Dashboard"
                                }
                                NavTab {
                                    to: Route::ClusterPage {},
                                    icon: &icondata::AiClusterOutlined,
                                    label: "Cluster"
                                }
                                NavTab {
                                    to: Route::StackPage {},
                                    icon: &icondata::AiThunderboltOutlined,
                                    label: "Stacks"
                                }
                                NavTab {
                                    to: Route::ProfilingPage {},
                                    icon: &icondata::AiSearchOutlined,
                                    label: "Profiling"
                                }
                                NavTab {
                                    to: Route::AnalyticsPage {},
                                    icon: &icondata::AiAreaChartOutlined,
                                    label: "Analytics"
                                }
                                NavTab {
                                    to: Route::PythonPage {},
                                    icon: &icondata::SiPython,
                                    label: "Python"
                                }
                                NavTab {
                                    to: Route::TracesPage {},
                                    icon: &icondata::AiApiOutlined,
                                    label: "Traces"
                                }
                    }
                    
                    // Right side controls
                    div {
                        class: "flex items-center space-x-4",
                        // Mobile menu button
                        button {
                            class: "md:hidden p-2 text-gray-500 hover:text-gray-700",
                            Icon {
                                icon: &icondata::AiMenuOutlined,
                                class: "w-5 h-5"
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
pub fn NavTab(to: Route, icon: &'static IconData, label: &'static str) -> Element {
    let route = use_route::<Route>();
    let is_active = route == to;
    
    let class_str = if is_active {
        "flex items-center space-x-2 px-3 py-2 rounded-lg text-sm font-medium transition-colors bg-blue-100 text-blue-700 hover:bg-blue-200"
    } else {
        "flex items-center space-x-2 px-3 py-2 rounded-lg text-sm font-medium transition-colors text-gray-700 hover:bg-gray-100 hover:text-gray-900"
    };
    
    rsx! {
        Link {
            to: to,
            class: class_str,
            Icon { icon, class: "w-4 h-4" }
            span {
                class: "hidden lg:inline",
                "{label}"
            }
        }
    }
}
