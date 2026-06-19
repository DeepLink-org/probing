use dioxus::prelude::*;

use crate::components::colors::colors;

#[component]
pub fn CollapsibleCardWithIcon(
    title: String,
    icon: Element,
    children: Element,
    #[props(optional)] badge: Option<String>,
    #[props(optional)] badge_classes: Option<String>,
    #[props(optional)] accent_border: Option<String>,
    #[props(optional, default = false)] default_open: bool,
) -> Element {
    let mut is_open = use_signal(|| default_open);
    let border_accent = accent_border.unwrap_or_else(|| "border-l-transparent".to_string());
    let badge_style = badge_classes.unwrap_or_else(|| {
        format!(
            "bg-{} text-{} border-{}",
            colors::CONTENT_ACCENT_BG,
            colors::CONTENT_ACCENT_TEXT,
            colors::CONTENT_ACCENT_BORDER
        )
    });

    rsx! {
        div {
            class: format!(
                "border border-gray-200 rounded-lg bg-white border-l-4 {border_accent} shadow-sm"
            ),
            div {
                class: format!(
                    "px-4 py-3 bg-{} border-b border-{} cursor-pointer hover:bg-{} transition-colors",
                    colors::CONTENT_BG,
                    colors::CONTENT_BORDER,
                    colors::BTN_SECONDARY_BG
                ),
                onclick: move |_| {
                    let current = *is_open.read();
                    *is_open.write() = !current;
                },
                div {
                    class: "flex items-center justify-between gap-3",
                    div {
                        class: "flex items-center gap-2 min-w-0",
                        {icon}
                        span {
                            class: "text-sm font-medium text-gray-900 font-mono truncate",
                            title: "{title}",
                            "{title}"
                        }
                        if let Some(label) = badge {
                            span {
                                class: "shrink-0 inline-flex items-center px-2 py-0.5 rounded-full text-[10px] font-semibold uppercase tracking-wide border {badge_style}",
                                "{label}"
                            }
                        }
                    }
                    div {
                        class: "transition-transform duration-200 shrink-0",
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
