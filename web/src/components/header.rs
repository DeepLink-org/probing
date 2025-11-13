use dioxus::prelude::*;
use dioxus_router::{Link, use_route, use_navigator};
use icondata::Icon as IconData;

use crate::app::{Route, PROFILING_VIEW};
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
                                ProfilingNavTab {}
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

/// Profiling 导航标签，带下拉菜单
#[component]
pub fn ProfilingNavTab() -> Element {
    let route = use_route::<Route>();
    let navigator = use_navigator();
    let is_active = route == Route::ProfilingPage {};
    let mut show_dropdown = use_signal(|| false);
    let current_view = PROFILING_VIEW.read();
    
    let view_label = match current_view.as_str() {
        "pprof" => "pprof",
        "torch" => "torch",
        "chrome-tracing" => "Timeline",
        _ => "Profiling",
    };
    
    let class_str = if is_active {
        "flex items-center space-x-2 px-3 py-2 rounded-lg text-sm font-medium transition-colors bg-blue-100 text-blue-700 hover:bg-blue-200 relative"
    } else {
        "flex items-center space-x-2 px-3 py-2 rounded-lg text-sm font-medium transition-colors text-gray-700 hover:bg-gray-100 hover:text-gray-900 relative"
    };
    
    rsx! {
        div {
            class: "relative",
            button {
                class: class_str,
                onclick: {
                    let mut show_dropdown = show_dropdown.clone();
                    let is_active = is_active;
                    move |_| {
                        if is_active {
                            let current = *show_dropdown.read();
                            *show_dropdown.write() = !current;
                        } else {
                            navigator.push(Route::ProfilingPage {});
                        }
                    }
                },
                Icon { icon: &icondata::AiSearchOutlined, class: "w-4 h-4" }
                span {
                    class: "hidden lg:inline",
                    if is_active {
                        "{view_label}"
                    } else {
                        "Profiling"
                    }
                }
                if is_active {
                    span {
                        class: "ml-1 text-xs",
                        if *show_dropdown.read() {
                            "▲"
                        } else {
                            "▼"
                        }
                    }
                }
            }
            
            // 下拉菜单
            if is_active && *show_dropdown.read() {
                div {
                    class: "absolute top-full left-0 mt-1 w-56 bg-white rounded-md shadow-lg border border-gray-200 z-50",
                    onclick: {
                        let mut show_dropdown = show_dropdown.clone();
                        move |_| *show_dropdown.write() = false
                    },
                    ProfilingDropdownItem {
                        view: "pprof".to_string(),
                        label: "pprof Flamegraph".to_string(),
                        icon: &icondata::CgPerformance,
                    }
                    ProfilingDropdownItem {
                        view: "torch".to_string(),
                        label: "torch Flamegraph".to_string(),
                        icon: &icondata::SiPytorch,
                    }
                    ProfilingDropdownItem {
                        view: "chrome-tracing".to_string(),
                        label: "Timeline".to_string(),
                        icon: &icondata::AiThunderboltOutlined,
                    }
                }
            }
        }
    }
}

#[component]
fn ProfilingDropdownItem(view: String, label: String, icon: &'static IconData) -> Element {
    let current_view = PROFILING_VIEW.read();
    let is_selected = *current_view == view;
    let mut show_dropdown = use_signal(|| false);
    
    rsx! {
        button {
            class: "w-full flex items-center space-x-2 px-4 py-2 text-sm text-left transition-colors",
            class: if is_selected {
                "bg-blue-50 text-blue-700 font-medium"
            } else {
                "text-gray-700 hover:bg-gray-50"
            },
            onclick: move |_| {
                *PROFILING_VIEW.write() = view.clone();
                *show_dropdown.write() = false;
            },
            Icon { icon, class: "w-4 h-4" }
            span { "{label}" }
            if is_selected {
                span { class: "ml-auto text-blue-600", "✓" }
            }
        }
    }
}
