//! Reusable card surfaces — used by agent chat, pages, and tool results.

use dioxus::prelude::*;

use crate::components::icon::Icon;

pub const SURFACE_BASE: &str =
    "rounded-lg border border-gray-200 bg-white shadow-sm overflow-hidden";

#[component]
pub fn SurfaceCard(children: Element) -> Element {
    rsx! {
        div { class: "{SURFACE_BASE}", {children} }
    }
}

#[component]
pub fn AccentSurface(accent: &'static str, children: Element) -> Element {
    rsx! {
        div { class: "border-l-4 {accent} {SURFACE_BASE}", {children} }
    }
}

#[component]
pub fn SurfaceCardHeader(
    title: String,
    #[props(optional)] subtitle: Option<String>,
    #[props(optional)] icon: Option<Element>,
    #[props(optional)] header_right: Option<Element>,
    #[props(default = "bg-gray-50/80")]
    header_class: &'static str,
) -> Element {
    rsx! {
        div { class: "px-3 py-2.5 border-b border-gray-100 {header_class}",
            div { class: "flex items-start gap-2 min-w-0",
                if let Some(ic) = icon {
                    div { class: "shrink-0 mt-0.5", {ic} }
                }
                div { class: "flex-1 min-w-0",
                    div { class: "text-sm font-semibold text-gray-900 truncate", title: "{title}",
                        "{title}"
                    }
                    if let Some(sub) = subtitle {
                        div { class: "text-[10px] text-gray-500 font-mono truncate mt-0.5", "{sub}" }
                    }
                }
                if let Some(right) = header_right {
                    div { class: "shrink-0 flex items-center gap-1", {right} }
                }
            }
        }
    }
}

#[component]
pub fn SurfaceCardBody(
    children: Element,
    #[props(default = "px-3 py-2")]
    class: &'static str,
) -> Element {
    rsx! {
        div { class: "{class}", {children} }
    }
}

#[component]
pub fn StatusBadge(label: &'static str, badge_class: &'static str) -> Element {
    rsx! {
        span {
            class: "inline-flex px-1.5 py-0.5 rounded text-[9px] font-semibold uppercase tracking-wide border {badge_class}",
            "{label}"
        }
    }
}

#[component]
pub fn ChipButton(
    label: String,
    disabled: bool,
    #[props(optional)] active: Option<bool>,
    onclick: EventHandler<()>,
) -> Element {
    let active = active.unwrap_or(false);
    let class = if active {
        "px-2 py-1 text-xs rounded-md border border-blue-300 bg-blue-100 text-blue-900 font-medium"
    } else {
        "px-2 py-1 text-xs rounded-md border border-gray-200 bg-gray-50 text-gray-700 hover:bg-blue-50 hover:border-blue-200 hover:text-blue-800"
    };
    rsx! {
        button {
            class: "{class} disabled:opacity-50 disabled:pointer-events-none transition-colors",
            disabled: disabled,
            onclick: move |_| onclick.call(()),
            "{label}"
        }
    }
}

/// Segmented width control (e.g. panel ⅓ / ⅔).
#[component]
pub fn WidthSegment(
    label: &'static str,
    selected: bool,
    title: &'static str,
    onclick: EventHandler<()>,
) -> Element {
    let class = if selected {
        "px-2 py-1 text-[10px] font-semibold rounded-md bg-white text-blue-700 shadow-sm border border-gray-200"
    } else {
        "px-2 py-1 text-[10px] font-medium rounded-md text-gray-500 hover:text-gray-800"
    };
    rsx! {
        button {
            class: "{class}",
            title: "{title}",
            onclick: move |_| onclick.call(()),
            "{label}"
        }
    }
}

#[component]
pub fn SurfaceIconHeader(
    icon: &'static icondata::Icon,
    icon_class: &'static str,
    title: String,
    subtitle: Option<String>,
    #[props(optional)] header_right: Option<Element>,
) -> Element {
    rsx! {
        SurfaceCardHeader {
            title: title,
            subtitle: subtitle,
            header_class: "bg-gradient-to-r from-slate-50 to-blue-50/40",
            icon: rsx! {
                Icon { icon, class: icon_class }
            },
            header_right: header_right,
        }
    }
}
