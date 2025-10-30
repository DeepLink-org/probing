use dioxus::prelude::*;
use dioxus_router::prelude::*;

use crate::app::Route;

#[component]
pub fn Header() -> Element {
    rsx! {
        header {
            class: "bg-white dark:bg-gray-800 shadow-sm border-b border-gray-200 dark:border-gray-700",
            div {
                class: "px-6 py-4",
                div {
                    class: "flex items-center justify-between",
                    // Logo and Brand
                    div {
                        class: "flex items-center space-x-4",
                                Link {
                                    to: Route::OverviewPage {},
                                    class: "text-xl font-bold text-gray-900 dark:text-white hover:text-blue-600 dark:hover:text-blue-400",
                                    "Probing Dashboard"
                                }
                    }
                    
                    // Top Navigation Tabs
                    nav {
                        class: "hidden md:flex items-center space-x-1",
                                NavTab {
                                    to: Route::OverviewPage {},
                                    icon: "📊",
                                    label: "Overview"
                                }
                                NavTab {
                                    to: Route::ClusterPage {},
                                    icon: "🖥️",
                                    label: "Cluster"
                                }
                                NavTab {
                                    to: Route::ActivityPage {},
                                    icon: "⚡",
                                    label: "Activity"
                                }
                                NavTab {
                                    to: Route::ProfilerPage {},
                                    icon: "🔍",
                                    label: "Profiler"
                                }
                                NavTab {
                                    to: Route::TimeseriesPage {},
                                    icon: "📈",
                                    label: "TimeSeries"
                                }
                                NavTab {
                                    to: Route::PythonPage {},
                                    icon: "🐍",
                                    label: "Inspect"
                                }
                    }
                    
                    // Right side controls
                    div {
                        class: "flex items-center space-x-4",
                        // Mobile menu button
                        button {
                            class: "md:hidden p-2 text-gray-500 hover:text-gray-700 dark:text-gray-400 dark:hover:text-gray-200",
                            "☰"
                        }
                        // Theme toggle
                        button {
                            class: "p-2 text-gray-500 hover:text-gray-700 dark:text-gray-400 dark:hover:text-gray-200",
                            "🌙"
                        }
                    }
                }
            }
        }
    }
}

#[component]
pub fn NavTab(to: Route, icon: &'static str, label: &'static str) -> Element {
    rsx! {
        Link {
            to: to,
            class: "flex items-center space-x-2 px-3 py-2 rounded-lg text-sm font-medium text-gray-700 dark:text-gray-300 hover:bg-gray-100 dark:hover:bg-gray-700 hover:text-gray-900 dark:hover:text-white transition-colors",
            span {
                class: "text-base",
                "{icon}"
            }
            span {
                class: "hidden lg:inline",
                "{label}"
            }
        }
    }
}
