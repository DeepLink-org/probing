use dioxus::prelude::*;

#[component]
pub fn CollapsibleCard(title: String, children: Element) -> Element {
    // Signal requires mut binding to call write()
    let mut is_open = use_signal(|| false);
    
    rsx! {
        div {
            class: "border border-gray-200 rounded-lg mb-2",
            div {
                class: "px-4 py-3 bg-gray-50 border-b border-gray-200 cursor-pointer hover:bg-gray-100 transition-colors",
                onclick: move |_| {
                    let current = *is_open.read();
                    *is_open.write() = !current;
                },
                div {
                    class: "flex items-center justify-between",
                    div {
                        class: "flex items-center space-x-2",
                        span {
                            class: "text-sm font-medium text-gray-900",
                            "{title}"
                        }
                    }
                    div {
                        class: "transition-transform duration-200",
                        class: if *is_open.read() { "rotate-180" } else { "rotate-0" },
                        svg {
                            class: "w-4 h-4 text-gray-500",
                            fill: "none",
                            stroke: "currentColor",
                            view_box: "0 0 24 24",
                            path {
                                stroke_linecap: "round",
                                stroke_linejoin: "round",
                                stroke_width: "2",
                                d: "M19 9l-7 7-7-7"
                            }
                        }
                    }
                }
            }
            if *is_open.read() {
                div {
                    class: "p-4",
                    {children}
                }
            }
        }
    }
}

#[component]
pub fn CollapsibleCardWithIcon(title: String, icon: Element, children: Element) -> Element {
    let mut is_open = use_signal(|| false);
    
    rsx! {
        div {
            class: "border border-gray-200 rounded-lg mb-2",
            div {
                class: "px-4 py-3 bg-gray-50 border-b border-gray-200 cursor-pointer hover:bg-gray-100 transition-colors",
                onclick: move |_| {
                    let current = *is_open.read();
                    *is_open.write() = !current;
                },
                div {
                    class: "flex items-center justify-between",
                    div {
                        class: "flex items-center space-x-2",
                        {icon}
                        span {
                            class: "text-sm font-medium text-gray-900",
                            "{title}"
                        }
                    }
                    div {
                        class: "transition-transform duration-200",
                        class: if *is_open.read() { "rotate-180" } else { "rotate-0" },
                        svg {
                            class: "w-4 h-4 text-gray-500",
                            fill: "none",
                            stroke: "currentColor",
                            view_box: "0 0 24 24",
                            path {
                                stroke_linecap: "round",
                                stroke_linejoin: "round",
                                stroke_width: "2",
                                d: "M19 9l-7 7-7-7"
                            }
                        }
                    }
                }
            }
            if *is_open.read() {
                div {
                    class: "p-4",
                    {children}
                }
            }
        }
    }
}
