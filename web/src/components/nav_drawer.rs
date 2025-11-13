use dioxus::prelude::*;
use dioxus_router::Link;
use crate::components::icon::Icon;

#[derive(Props, Clone, PartialEq)]
pub struct NavDrawerProps {
    pub selected_tab: Signal<String>,
    pub pprof_freq: Signal<i32>,
    pub torch_enabled: Signal<bool>,
    pub on_tab_change: EventHandler<String>,
    pub on_pprof_freq_change: EventHandler<i32>,
    pub on_torch_toggle: EventHandler<bool>,
}

#[component]
pub fn NavDrawer(props: NavDrawerProps) -> Element {
    rsx! {
        div {
            class: "w-64 bg-white border-r border-gray-200 h-full",
            div {
                class: "p-4",
                h2 {
                    class: "text-lg font-semibold text-gray-900 mb-4",
                    "Profiler Settings"
                }
                
                // Profiler Tabs
                div {
                    class: "space-y-2 mb-6",
                    NavItem {
                        icon: rsx! { Icon { icon: &icondata::CgPerformance } },
                        label: "pprof flamegraph",
                        value: "pprof",
                        is_selected: *props.selected_tab.read() == "pprof",
                        onclick: move |_| props.on_tab_change.call("pprof".to_string())
                    }
                    
                    NavItem {
                        icon: rsx! { Icon { icon: &icondata::SiPytorch } },
                        label: "torch flamegraph",
                        value: "torch",
                        is_selected: *props.selected_tab.read() == "torch",
                        onclick: move |_| props.on_tab_change.call("torch".to_string())
                    }
                }
                
                // Chrome Tracing Link
                div {
                    class: "mt-4 pt-4 border-t border-gray-200",
                    Link {
                        to: crate::app::Route::ChromeTracingPage {},
                        class: "w-full flex items-center space-x-3 px-3 py-2 text-sm font-medium rounded-md transition-colors text-gray-600 hover:bg-gray-100 hover:text-gray-900",
                        Icon { icon: &icondata::AiThunderboltOutlined, class: "w-5 h-5" }
                        span { "Chrome Tracing" }
                    }
                }
                
                // Settings Section
                div {
                    class: "space-y-4",
                    h3 {
                        class: "text-md font-medium text-gray-700",
                        "Settings"
                    }
                    
                    // Pprof sample frequency slider (discrete: 0,10,100,1000). 0 means OFF.
                    div {
                        class: "space-y-1",
                        {
                            // compute current index based on freq
                            let freq = *props.pprof_freq.read();
                            let current_idx = if freq <= 0 { 0 } else if freq <= 10 { 1 } else if freq <= 100 { 2 } else { 3 };
                            let label = match current_idx { 0 => 0, 1 => 10, 2 => 100, _ => 1000 };
                            rsx!{
                                div { class: "flex items-center justify-between",
                                    span { class: "text-sm text-gray-600", "Pprof Frequency" }
                                    span { class: "text-sm text-gray-800 font-mono", "{label} Hz" }
                                }
                                input {
                                    r#type: "range",
                                    key: "pprof-slider-{current_idx}",
                                    min: "0",
                                    max: "3",
                                    step: "1",
                                    value: "{current_idx}",
                                    oninput: move |ev| {
                                        if let Ok(idx) = ev.value().parse::<i32>() {
                                            let mapped = match idx { 0 => 0, 1 => 10, 2 => 100, _ => 1000 };
                                            props.on_pprof_freq_change.call(mapped);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    
                    // Torch Settings
                    div {
                        class: "flex items-center justify-between",
                        span {
                            class: "text-sm text-gray-600",
                            "Torch"
                        }
                        Switch {
                            checked: *props.torch_enabled.read(),
                            onchange: move |checked| props.on_torch_toggle.call(checked)
                        }
                    }
                }
                
                // Github Link
                div {
                    class: "mt-8 pt-4 border-t border-gray-200",
                    a {
                        href: "https://github.com/reiase/probing",
                        target: "_blank",
                        class: "flex items-center space-x-2 text-sm text-gray-600 hover:text-gray-900",
                        Icon { icon: &icondata::AiGithubOutlined, class: "w-4 h-4" }
                        span { "Github" }
                    }
                }
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct NavItemProps {
    pub icon: Element,
    pub label: String,
    pub value: String,
    pub is_selected: bool,
    pub onclick: EventHandler<()>,
}

#[component]
pub fn NavItem(props: NavItemProps) -> Element {
    rsx! {
        button {
            class: "w-full flex items-center space-x-3 px-3 py-2 text-sm font-medium rounded-md transition-colors",
            class: if props.is_selected {
                "bg-blue-100 text-blue-700"
            } else {
                "text-gray-600 hover:bg-gray-100 hover:text-gray-900"
            },
            onclick: move |_| props.onclick.call(()),
            {props.icon}
            span { "{props.label}" }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct SwitchProps {
    pub checked: bool,
    pub onchange: EventHandler<bool>,
}

#[component]
pub fn Switch(props: SwitchProps) -> Element {
    rsx! {
        button {
            class: "relative inline-flex h-6 w-11 items-center rounded-full transition-colors",
            class: if props.checked {
                "bg-blue-600"
            } else {
                "bg-gray-200"
            },
            onclick: move |_| props.onchange.call(!props.checked),
            span {
                class: "inline-block h-4 w-4 transform rounded-full bg-white transition-transform",
                class: if props.checked {
                    "translate-x-6"
                } else {
                    "translate-x-1"
                }
            }
        }
    }
}
