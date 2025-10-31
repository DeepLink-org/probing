use dioxus::prelude::*;

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
            class: "w-64 bg-white dark:bg-gray-800 border-r border-gray-200 dark:border-gray-700 h-full",
            div {
                class: "p-4",
                h2 {
                    class: "text-lg font-semibold text-gray-900 dark:text-white mb-4",
                    "Profiler Settings"
                }
                
                // Profiler Tabs
                div {
                    class: "space-y-2 mb-6",
                    NavItem {
                        icon: rsx! {
                            svg {
                                class: "w-5 h-5",
                                view_box: "0 0 24 24",
                                fill: "currentColor",
                                path {
                                    d: "M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm-2 15l-5-5 1.41-1.41L10 14.17l7.59-7.59L19 8l-9 9z"
                                }
                            }
                        },
                        label: "Pprof Profiling",
                        value: "pprof",
                        is_selected: *props.selected_tab.read() == "pprof",
                        onclick: move |_| props.on_tab_change.call("pprof".to_string())
                    }
                    
                    NavItem {
                        icon: rsx! {
                            svg {
                                class: "w-5 h-5",
                                view_box: "0 0 24 24",
                                fill: "currentColor",
                                path {
                                    d: "M12 2l3.09 6.26L22 9.27l-5 4.87 1.18 6.88L12 17.77l-6.18 3.25L7 14.14 2 9.27l6.91-1.01L12 2z"
                                }
                            }
                        },
                        label: "Torch Profiling",
                        value: "torch",
                        is_selected: *props.selected_tab.read() == "torch",
                        onclick: move |_| props.on_tab_change.call("torch".to_string())
                    }
                }
                
                // Settings Section
                div {
                    class: "space-y-4",
                    h3 {
                        class: "text-md font-medium text-gray-700 dark:text-gray-300",
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
                                    span { class: "text-sm text-gray-600 dark:text-gray-400", "Pprof Frequency" }
                                    span { class: "text-sm text-gray-800 dark:text-gray-200 font-mono", "{label} Hz" }
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
                            class: "text-sm text-gray-600 dark:text-gray-400",
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
                    class: "mt-8 pt-4 border-t border-gray-200 dark:border-gray-700",
                    a {
                        href: "https://github.com/reiase/probing",
                        target: "_blank",
                        class: "flex items-center space-x-2 text-sm text-gray-600 dark:text-gray-400 hover:text-gray-900 dark:hover:text-white",
                        svg {
                            class: "w-4 h-4",
                            view_box: "0 0 24 24",
                            fill: "currentColor",
                            path {
                                d: "M12 0c-6.626 0-12 5.373-12 12 0 5.302 3.438 9.8 8.207 11.387.599.111.793-.261.793-.577v-2.234c-3.338.726-4.033-1.416-4.033-1.416-.546-1.387-1.333-1.756-1.333-1.756-1.089-.745.083-.729.083-.729 1.205.084 1.839 1.237 1.839 1.237 1.07 1.834 2.807 1.304 3.492.997.107-.775.418-1.305.762-1.604-2.665-.305-5.467-1.334-5.467-5.931 0-1.311.469-2.381 1.236-3.221-.124-.303-.535-1.524.117-3.176 0 0 1.008-.322 3.301 1.23.957-.266 1.983-.399 3.003-.404 1.02.005 2.047.138 3.006.404 2.291-1.552 3.297-1.23 3.297-1.23.653 1.653.242 2.874.118 3.176.77.84 1.235 1.911 1.235 3.221 0 4.609-2.807 5.624-5.479 5.921.43.372.823 1.102.823 2.222v3.293c0 .319.192.694.801.576 4.765-1.589 8.199-6.086 8.199-11.386 0-6.627-5.373-12-12-12z"
                            }
                        }
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
                "bg-blue-100 text-blue-700 dark:bg-blue-900 dark:text-blue-200"
            } else {
                "text-gray-600 dark:text-gray-400 hover:bg-gray-100 dark:hover:bg-gray-700 hover:text-gray-900 dark:hover:text-white"
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
                "bg-gray-200 dark:bg-gray-700"
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
