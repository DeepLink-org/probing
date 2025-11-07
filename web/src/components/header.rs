use dioxus::prelude::*;
use dioxus_router::Link;
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
                                    to: Route::OverviewPage {},
                                    class: "text-xl font-bold text-gray-900 hover:text-blue-600",
                                    "Probing Dashboard"
                                }
                    }
                    
                    // Top Navigation Tabs
                    nav {
                        class: "hidden md:flex items-center space-x-1",
                                NavTab {
                                    to: Route::OverviewPage {},
                                    icon: &icondata::AiLineChartOutlined,
                                    label: "Overview"
                                }
                                NavTab {
                                    to: Route::ClusterPage {},
                                    icon: &icondata::AiClusterOutlined,
                                    label: "Cluster"
                                }
                                NavTab {
                                    to: Route::ActivityPage {},
                                    icon: &icondata::AiThunderboltOutlined,
                                    label: "Activity"
                                }
                                NavTab {
                                    to: Route::ProfilerPage {},
                                    icon: &icondata::AiSearchOutlined,
                                    label: "Profiler"
                                }
                                NavTab {
                                    to: Route::TimeseriesPage {},
                                    icon: &icondata::AiAreaChartOutlined,
                                    label: "TimeSeries"
                                }
                                NavTab {
                                    to: Route::PythonPage {},
                                    icon: &icondata::SiPython,
                                    label: "Inspect"
                                }
                                NavTab {
                                    to: Route::TracingPage {},
                                    icon: &icondata::AiApiOutlined,
                                    label: "Tracing"
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
    rsx! {
        Link {
            to: to,
            class: "flex items-center space-x-2 px-3 py-2 rounded-lg text-sm font-medium text-gray-700 hover:bg-gray-100 hover:text-gray-900 transition-colors",
            Icon { icon, class: "w-4 h-4" }
            span {
                class: "hidden lg:inline",
                "{label}"
            }
        }
    }
}
