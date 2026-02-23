use dioxus::prelude::*;

use crate::components::colors::colors;

#[component]
pub fn CollapsibleCardWithIcon(title: String, icon: Element, children: Element) -> Element {
    let mut is_open = use_signal(|| false);

    rsx! {
        div {
            class: "border border-gray-200 rounded-lg mb-2 bg-white",
            div {
                class: format!("px-4 py-3 bg-{} border-b border-{} cursor-pointer hover:bg-{} transition-colors", colors::CONTENT_BG, colors::CONTENT_BORDER, colors::BTN_SECONDARY_BG),
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
