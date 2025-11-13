use dioxus::prelude::*;
use dioxus_router::{Link, use_route, use_navigator};
use icondata::Icon as IconData;
use web_sys::window;

use crate::app::{Route, PROFILING_VIEW, PROFILING_PPROF_FREQ, PROFILING_TORCH_ENABLED, 
    PROFILING_CHROME_DATA_SOURCE, PROFILING_CHROME_LIMIT, PROFILING_PYTORCH_STEPS,
    SIDEBAR_WIDTH, SIDEBAR_HIDDEN};
use crate::components::icon::Icon;
use crate::components::colors::colors;
use crate::api::ApiClient;

#[component]
pub fn Sidebar() -> Element {
    let route = use_route::<Route>();
    let mut show_profiling_dropdown = use_signal(|| false);
    let mut is_resizing = use_signal(|| false);
    let mut drag_start_x = use_signal(|| 0.0);
    let mut drag_start_width = use_signal(|| 256.0);
    
    // 从 localStorage 加载初始状态
    use_effect(move || {
        if let Some(window) = window() {
            let storage = window.local_storage().ok().flatten();
            if let Some(storage) = storage {
                if let Ok(Some(width_str)) = storage.get_item("sidebar_width") {
                    if let Ok(width) = width_str.parse::<f64>() {
                        if width >= 200.0 && width <= 600.0 {
                            *SIDEBAR_WIDTH.write() = width;
                        }
                    }
                }
                if let Ok(Some(hidden_str)) = storage.get_item("sidebar_hidden") {
                    if hidden_str == "true" {
                        *SIDEBAR_HIDDEN.write() = true;
                    }
                }
            }
        }
    });
    
    // 保存状态到 localStorage
    let save_state = move || {
        if let Some(window) = window() {
            let storage = window.local_storage().ok().flatten();
            if let Some(storage) = storage {
                let _ = storage.set_item("sidebar_width", &SIDEBAR_WIDTH.read().to_string());
                let _ = storage.set_item("sidebar_hidden", &SIDEBAR_HIDDEN.read().to_string());
            }
        }
    };
    
    // 注意：拖拽调整宽度使用 onmousemove/onmouseup 事件处理
    
    let sidebar_width = SIDEBAR_WIDTH.read();
    let sidebar_hidden = SIDEBAR_HIDDEN.read();
    
    // 预计算颜色类名
    let aside_class = format!("bg-gradient-to-b from-{} via-{} to-{} border-r border-{} h-screen flex flex-col flex-shrink-0 shadow-xl",
        colors::SIDEBAR_BG, colors::SIDEBAR_BG_VIA, colors::SIDEBAR_BG, colors::SIDEBAR_BORDER);
    let logo_border_class = format!("px-6 py-4 border-b border-{}", colors::SIDEBAR_BORDER);
    let brand_title_class = format!("text-lg font-bold text-{}", colors::SIDEBAR_TEXT_PRIMARY);
    let brand_subtitle_class = format!("text-xs text-{}", colors::SIDEBAR_TEXT_MUTED);
    let section_title_class = format!("px-3 py-2 text-xs font-semibold text-{} uppercase tracking-wider", colors::SIDEBAR_TEXT_MUTED);
    let footer_class = format!("px-6 py-4 border-t border-{}", colors::SIDEBAR_BORDER);
    let footer_link_class = format!("flex items-center space-x-2 text-sm text-{} hover:text-{} transition-colors",
        colors::SIDEBAR_TEXT_MUTED, colors::PRIMARY_TEXT_DARK);
    let hide_button_class = format!("absolute top-4 -right-3 w-6 h-6 bg-{} border border-{} rounded-full shadow-lg flex items-center justify-center hover:bg-{} z-30 transition-colors",
        colors::SIDEBAR_ACTIVE_BG, "slate-700", "slate-600");
    let hide_icon_class = format!("w-4 h-4 text-{}", colors::SIDEBAR_TEXT_SECONDARY);
    
    rsx! {
        div {
            class: "relative flex h-screen",
            style: format!("width: {}px;", *sidebar_width),
            aside {
                class: "{aside_class}",
                style: format!("width: {}px;", *sidebar_width),
                // Logo and Brand
                div {
                    class: "{logo_border_class}",
                    Link {
                    to: Route::DashboardPage {},
                    class: "flex items-center space-x-3",
                    img {
                        src: "/assets/logo.svg",
                        alt: "Probing Logo",
                        class: "w-8 h-8 flex-shrink-0",
                    }
                    div {
                        class: "flex flex-col",
                        span {
                            class: "{brand_title_class}",
                            "Probing"
                        }
                        span {
                            class: "{brand_subtitle_class}",
                            "Performance Profiler"
                        }
                    }
                }
            }
            
            // Navigation
            nav {
                class: "flex-1 overflow-y-auto py-4",
                div {
                    class: "px-3 space-y-1",
                    // Overview Section
                    div {
                        class: "mb-4",
                        div {
                            class: "{section_title_class}",
                            "Overview"
                        }
                        SidebarNavItem {
                            to: Route::DashboardPage {},
                            icon: &icondata::AiLineChartOutlined,
                            label: "Dashboard",
                            is_active: route == Route::DashboardPage {},
                        }
                    }
                    
                    // Analysis Section
                    div {
                        class: "mb-4",
                        div {
                            class: "{section_title_class}",
                            "Analysis"
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
                    }
                    
                    // System Section
                    div {
                        class: "mb-4",
                        div {
                            class: "{section_title_class}",
                            "System"
                        }
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
            }
            
            // Footer
            div {
                class: "{footer_class}",
                a {
                    href: "https://github.com/reiase/probing",
                    target: "_blank",
                    class: "{footer_link_class}",
                    Icon { icon: &icondata::AiGithubOutlined, class: "w-4 h-4" }
                    span { "GitHub" }
                }
            }
            }
            
            // 隐藏/显示按钮
            button {
                class: "{hide_button_class}",
                title: "Hide Sidebar",
                onclick: move |_| {
                    *SIDEBAR_HIDDEN.write() = true;
                    save_state();
                },
                Icon {
                    icon: &icondata::AiMenuFoldOutlined,
                    class: "w-4 h-4 text-slate-300"
                }
            }
            
            // 拖拽调整宽度手柄
            {
                let hover_class = format!("hover:bg-{}/50", colors::PRIMARY);
                let active_class = if *is_resizing.read() {
                    format!("bg-{}", colors::PRIMARY)
                } else {
                    "bg-transparent".to_string()
                };
                let drag_handle_class = format!("absolute top-0 right-0 w-1 h-full cursor-col-resize {} transition-colors group z-20 {}", hover_class, active_class);
                rsx! {
                    div {
                        class: "{drag_handle_class}",
                        onmousedown: move |ev| {
                            *is_resizing.write() = true;
                            *drag_start_x.write() = ev.element_coordinates().x as f64;
                            *drag_start_width.write() = *SIDEBAR_WIDTH.read();
                            ev.prevent_default();
                        },
                        onmousemove: move |ev| {
                            if *is_resizing.read() {
                                let current_x = ev.element_coordinates().x as f64;
                                let delta_x = current_x - *drag_start_x.read();
                                let new_width = (*drag_start_width.read() + delta_x).max(200.0).min(600.0);
                                *SIDEBAR_WIDTH.write() = new_width;
                            }
                        },
                        onmouseup: move |_| {
                            if *is_resizing.read() {
                                *is_resizing.write() = false;
                                // 保存到 localStorage
                                if let Some(window) = window() {
                                    let storage = window.local_storage().ok().flatten();
                                    if let Some(storage) = storage {
                                        let _ = storage.set_item("sidebar_width", &SIDEBAR_WIDTH.read().to_string());
                                    }
                                }
                            }
                        },
                        onmouseleave: move |_| {
                            if *is_resizing.read() {
                                *is_resizing.write() = false;
                            }
                        },
                        div {
                            class: "absolute top-1/2 right-0 transform translate-x-1/2 -translate-y-1/2 w-1 h-8 bg-gray-300 rounded-full opacity-0 group-hover:opacity-100 transition-opacity",
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn SidebarNavItem(to: Route, icon: &'static IconData, label: &'static str, is_active: bool) -> Element {
    // 统一颜色系统：深色背景下的激活和未激活状态
    let class_str = if is_active {
        format!("flex items-center space-x-3 px-3 py-2 text-sm font-medium rounded-md bg-{} text-{} border-l-2 border-{} shadow-sm",
            colors::PRIMARY_BG, colors::PRIMARY_TEXT, colors::PRIMARY_BORDER)
    } else {
        format!("flex items-center space-x-3 px-3 py-2 text-sm font-medium rounded-md text-{} hover:bg-{} hover:text-{} transition-colors",
            colors::SIDEBAR_TEXT_SECONDARY, colors::SIDEBAR_HOVER_BG, colors::PRIMARY_TEXT)
    };
    
    rsx! {
        Link {
            to: to,
            class: "{class_str}",
            Icon { icon, class: "w-5 h-5" }
            span { "{label}" }
        }
    }
}

/// Profiling 侧边栏项，带折叠子菜单
#[component]
fn ProfilingSidebarItem(show_dropdown: Signal<bool>) -> Element {
    let route = use_route::<Route>();
    let is_active = route == Route::ProfilingPage {};
    
    rsx! {
        div {
            // 主菜单项 - 只负责展开/收起
            {
                let button_class = if is_active {
                    format!("w-full flex items-center px-3 py-2 text-sm font-medium rounded-md transition-colors bg-{} text-{} border-l-2 border-{} shadow-sm",
                        colors::PRIMARY_BG, colors::PRIMARY_TEXT, colors::PRIMARY_BORDER)
                } else {
                    format!("w-full flex items-center px-3 py-2 text-sm font-medium rounded-md transition-colors text-{} hover:bg-{} hover:text-{}",
                        colors::SIDEBAR_TEXT_SECONDARY, colors::SIDEBAR_HOVER_BG, colors::PRIMARY_TEXT)
                };
                rsx! {
                    button {
                        class: "{button_class}",
                        onclick: {
                            let mut show_dropdown = show_dropdown.clone();
                            move |_| {
                                // 简单的折叠/展开逻辑
                                let current = *show_dropdown.read();
                                *show_dropdown.write() = !current;
                            }
                        },
                        div {
                            class: "flex items-center space-x-3",
                            Icon { icon: &icondata::AiSearchOutlined, class: "w-5 h-5" }
                            span { "Profiling" }
                        }
                    }
                }
            }
            
            // 子菜单 - 根据展开状态显示
            if *show_dropdown.read() {
                div {
                    class: "ml-6 mt-1 space-y-1",
                    ProfilingSubItem {
                        view: "pprof".to_string(),
                        label: "pprof Flamegraph".to_string(),
                        icon: &icondata::CgPerformance,
                    }
                    ProfilingSubItem {
                        view: "torch".to_string(),
                        label: "torch Flamegraph".to_string(),
                        icon: &icondata::SiPytorch,
                    }
                    ProfilingSubItem {
                        view: "chrome-tracing".to_string(),
                        label: "Timeline".to_string(),
                        icon: &icondata::AiThunderboltOutlined,
                    }
                    
                    // Profiling 控制面板（仅在 Profiling 页面时显示）
                    if is_active {
                        ProfilingControlsPanel {}
                    }
                }
            }
        }
    }
}

#[component]
fn ProfilingSubItem(view: String, label: String, icon: &'static IconData) -> Element {
    let route = use_route::<Route>();
    let navigator = use_navigator();
    let current_view = PROFILING_VIEW.read();
    let is_selected = *current_view == view;
    let is_on_profiling_page = route == Route::ProfilingPage {};
    
    // 统一颜色系统：深色背景下的选中和未选中状态
    let button_class = if is_selected {
        format!("w-full flex items-center space-x-2 px-3 py-2 text-sm rounded-md transition-colors bg-{} text-{} font-medium border-l-2 border-{} shadow-sm",
            colors::PRIMARY_BG, colors::PRIMARY_TEXT, colors::PRIMARY_BORDER)
    } else {
        format!("w-full flex items-center space-x-2 px-3 py-2 text-sm rounded-md transition-colors text-{} hover:bg-{} hover:text-{}",
            colors::SIDEBAR_TEXT_SECONDARY, colors::SIDEBAR_HOVER_BG, colors::PRIMARY_TEXT)
    };
    
    rsx! {
        button {
            class: "{button_class}",
            onclick: {
                let view_clone = view.clone();
                let navigator = navigator.clone();
                let is_on_profiling = is_on_profiling_page;
                move |_| {
                    // 设置视图
                    *PROFILING_VIEW.write() = view_clone.clone();
                    // 如果不在 Profiling 页面，导航到 Profiling 页面
                    if !is_on_profiling {
                        navigator.push(Route::ProfilingPage {});
                    }
                }
            },
            Icon { icon, class: "w-4 h-4" }
            span { "{label}" }
            if is_selected {
                // 使用统一的选中指示器
                {
                    let checkmark_class = format!("ml-auto text-{} font-semibold", colors::PRIMARY_TEXT_DARK);
                    rsx! {
                        span { class: "{checkmark_class}", "✓" }
                    }
                }
            }
        }
    }
}

/// Profiling 控制面板（在侧边栏中显示）
#[component]
fn ProfilingControlsPanel() -> Element {
    let current_view = PROFILING_VIEW.read();
    let panel_border_class = format!("mt-4 pt-4 border-t border-{}", colors::SIDEBAR_BORDER);
    let control_title_class = format!("text-xs font-semibold text-{}", colors::SIDEBAR_TEXT_SECONDARY);
    let control_value_class = format!("text-xs text-{}", colors::SIDEBAR_TEXT_MUTED);
    let toggle_enabled_class = format!("relative inline-flex h-6 w-11 items-center rounded-full transition-colors w-full bg-{}", colors::PRIMARY);
    let toggle_disabled_class = format!("relative inline-flex h-6 w-11 items-center rounded-full transition-colors w-full bg-{}", colors::SIDEBAR_ACTIVE_BG);
    let toggle_label_class = format!("ml-2 text-xs text-{}", colors::SIDEBAR_TEXT_SECONDARY);
    let button_active_class = format!("flex-1 px-2 py-1 text-xs font-medium rounded bg-{} text-white shadow-sm", colors::PRIMARY);
    let button_inactive_class = format!("flex-1 px-2 py-1 text-xs font-medium rounded bg-{} text-{} hover:bg-{}", colors::SIDEBAR_ACTIVE_BG, colors::SIDEBAR_TEXT_SECONDARY, "slate-600");
    let input_class = format!("w-full px-2 py-1 border border-{} bg-{} text-{} rounded text-xs focus:border-{} focus:outline-none",
        colors::SIDEBAR_INPUT_BORDER, colors::SIDEBAR_INPUT_BG, colors::SIDEBAR_TEXT_SECONDARY, colors::PRIMARY_BORDER);
    
    rsx! {
        div {
            class: "{panel_border_class}",
            div {
                class: "px-3 space-y-4",
                // pprof 控制
                if *current_view == "pprof" {
                    {
                        let freq = *PROFILING_PPROF_FREQ.read();
                        let current_idx = if freq <= 0 { 0 } else if freq <= 10 { 1 } else if freq <= 100 { 2 } else { 3 };
                        let label = match current_idx { 0 => 0, 1 => 10, 2 => 100, _ => 1000 };
                        rsx! {
                            div {
                                class: "space-y-2",
                                div {
                                    class: "{control_title_class}",
                                    "Pprof Frequency"
                                }
                                div {
                                    class: "space-y-1",
                                    div {
                                        class: "{control_value_class} flex items-center justify-between",
                                        span { "{label} Hz" }
                                    }
                                    input {
                                        r#type: "range",
                                        min: "0",
                                        max: "3",
                                        step: "1",
                                        value: "{current_idx}",
                                        class: "w-full",
                                        oninput: move |ev| {
                                            if let Ok(idx) = ev.value().parse::<i32>() {
                                                let mapped = match idx { 0 => 0, 1 => 10, 2 => 100, _ => 1000 };
                                                *PROFILING_PPROF_FREQ.write() = mapped;
                                                spawn(async move {
                                                    let client = ApiClient::new();
                                                    let expr = if mapped <= 0 { 
                                                        "set probing.pprof.sample_freq=;".to_string() 
                                                    } else { 
                                                        format!("set probing.pprof.sample_freq={};", mapped) 
                                                    };
                                                    let _ = client.execute_query(&expr).await;
                                                });
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                
                // torch 控制
                if *current_view == "torch" {
                    div {
                        class: "space-y-2",
                        div {
                            class: "{control_title_class}",
                            "Torch Profiling"
                        }
                        {
                            let toggle_class = if *PROFILING_TORCH_ENABLED.read() {
                                toggle_enabled_class.clone()
                            } else {
                                toggle_disabled_class.clone()
                            };
                            rsx! {
                                button {
                                    class: "{toggle_class}",
                                    onclick: move |_| {
                                        let enabled = !*PROFILING_TORCH_ENABLED.read();
                                        spawn(async move {
                                            let client = ApiClient::new();
                                            let expr = if enabled {
                                                "set probing.torch.profiling=on;".to_string()
                                            } else {
                                                "set probing.torch.profiling=;".to_string()
                                            };
                                            let _ = client.execute_query(&expr).await;
                                            *PROFILING_TORCH_ENABLED.write() = enabled;
                                        });
                                    },
                                    span {
                                        class: "inline-block h-4 w-4 transform rounded-full bg-white transition-transform",
                                        class: if *PROFILING_TORCH_ENABLED.read() {
                                            "translate-x-6"
                                        } else {
                                            "translate-x-1"
                                        }
                                    }
                                    span {
                                        class: "{toggle_label_class}",
                                        if *PROFILING_TORCH_ENABLED.read() {
                                            "Enabled"
                                        } else {
                                            "Disabled"
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                
                // Chrome Tracing 控制
                if *current_view == "chrome-tracing" {
                    div {
                        class: "space-y-3",
                        div {
                            class: "{control_title_class}",
                            "Data Source"
                        }
                        div {
                            class: "flex gap-1",
                            {
                                let trace_btn_class = if *PROFILING_CHROME_DATA_SOURCE.read() == "trace" {
                                    button_active_class.clone()
                                } else {
                                    button_inactive_class.clone()
                                };
                                rsx! {
                                    button {
                                        class: "{trace_btn_class}",
                                        onclick: move |_| *PROFILING_CHROME_DATA_SOURCE.write() = "trace".to_string(),
                                        "Trace"
                                    }
                                }
                            }
                            {
                                let pytorch_btn_class = if *PROFILING_CHROME_DATA_SOURCE.read() == "pytorch" {
                                    button_active_class.clone()
                                } else {
                                    button_inactive_class.clone()
                                };
                                rsx! {
                                    button {
                                        class: "{pytorch_btn_class}",
                                        onclick: move |_| *PROFILING_CHROME_DATA_SOURCE.write() = "pytorch".to_string(),
                                        "PyTorch"
                                    }
                                }
                            }
                        }
                        
                        // Trace Events 控制
                        if *PROFILING_CHROME_DATA_SOURCE.read() == "trace" {
                            div {
                                class: "space-y-1",
                                div {
                                    class: "{control_title_class}",
                                    "Event Limit"
                                }
                                div {
                                    class: "flex items-center gap-2",
                                    span {
                                        class: "{control_value_class}",
                                        "{*PROFILING_CHROME_LIMIT.read()}"
                                    }
                                    input {
                                        r#type: "range",
                                        min: "100",
                                        max: "5000",
                                        step: "100",
                                        value: "{*PROFILING_CHROME_LIMIT.read()}",
                                        class: "flex-1",
                                        oninput: move |ev| {
                                            if let Ok(val) = ev.value().parse::<usize>() {
                                                *PROFILING_CHROME_LIMIT.write() = val;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        
                        // PyTorch Profiler 控制
                        if *PROFILING_CHROME_DATA_SOURCE.read() == "pytorch" {
                            div {
                                class: "space-y-2",
                                div {
                                    class: "{control_title_class}",
                                    "Steps"
                                }
                                input {
                                    r#type: "number",
                                    min: "1",
                                    max: "100",
                                    value: "{*PROFILING_PYTORCH_STEPS.read()}",
                                    class: "{input_class}",
                                    oninput: move |ev| {
                                        if let Ok(val) = ev.value().parse::<i32>() {
                                            *PROFILING_PYTORCH_STEPS.write() = val.max(1).min(100);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
