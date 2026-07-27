use dioxus::prelude::*;

use crate::components::icon::Icon;
use crate::ui_version::{href_for, UiVersion};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FindingTone {
    Critical,
    Warning,
    Info,
    Healthy,
}

impl FindingTone {
    fn classes(self) -> &'static str {
        match self {
            Self::Critical => "border-red-200 bg-red-50 text-red-950",
            Self::Warning => "border-amber-200 bg-amber-50 text-amber-950",
            Self::Info => "border-blue-200 bg-blue-50 text-blue-950",
            Self::Healthy => "border-emerald-200 bg-emerald-50 text-emerald-950",
        }
    }

    fn badge(self) -> &'static str {
        match self {
            Self::Critical => "bg-red-100 text-red-700",
            Self::Warning => "bg-amber-100 text-amber-800",
            Self::Info => "bg-blue-100 text-blue-700",
            Self::Healthy => "bg-emerald-100 text-emerald-700",
        }
    }
}

#[component]
pub fn NextPageHeader(
    title: String,
    subtitle: String,
    #[props(optional)] actions: Option<Element>,
) -> Element {
    rsx! {
        header { class: "flex flex-wrap items-start justify-between gap-3",
            div { class: "min-w-0",
                h1 { class: "text-2xl font-semibold tracking-tight text-gray-950", "{title}" }
                p { class: "mt-1 max-w-3xl text-sm text-gray-500", "{subtitle}" }
            }
            if let Some(actions) = actions {
                div { class: "flex shrink-0 flex-wrap items-center gap-2", {actions} }
            }
        }
    }
}

#[component]
pub fn SectionCard(
    title: String,
    children: Element,
    #[props(optional)] subtitle: Option<String>,
    #[props(optional)] actions: Option<Element>,
    #[props(default = String::from("p-4"))] body_class: String,
    #[props(default = false)] fill: bool,
) -> Element {
    let section_class = if fill {
        "flex h-full min-h-0 flex-col overflow-hidden rounded-xl border border-gray-200 bg-white shadow-sm"
    } else {
        "overflow-hidden rounded-xl border border-gray-200 bg-white shadow-sm"
    };
    rsx! {
        section { class: "{section_class}",
            div { class: "flex flex-wrap items-start justify-between gap-3 border-b border-gray-100 px-4 py-3",
                div {
                    h2 { class: "text-sm font-semibold text-gray-950", "{title}" }
                    if let Some(subtitle) = subtitle {
                        p { class: "mt-0.5 text-xs text-gray-500", "{subtitle}" }
                    }
                }
                if let Some(actions) = actions {
                    div { class: "flex items-center gap-2", {actions} }
                }
            }
            div { class: "{body_class}", {children} }
        }
    }
}

#[component]
pub fn MetricCard(
    label: String,
    value: String,
    #[props(optional)] detail: Option<String>,
    #[props(optional)] icon: Option<&'static icondata::Icon>,
) -> Element {
    rsx! {
        div { class: "rounded-xl border border-gray-200 bg-white px-4 py-3 shadow-sm",
            div { class: "flex items-center gap-2 text-xs font-medium uppercase tracking-wide text-gray-500",
                if let Some(icon) = icon {
                    Icon { icon, class: "h-4 w-4 text-blue-600" }
                }
                "{label}"
            }
            div { class: "mt-2 text-2xl font-semibold tabular-nums text-gray-950", "{value}" }
            if let Some(detail) = detail {
                div { class: "mt-1 text-xs text-gray-500", "{detail}" }
            }
        }
    }
}

#[component]
pub fn FindingCard(
    eyebrow: String,
    title: String,
    detail: String,
    tone: FindingTone,
    #[props(optional)] action: Option<Element>,
) -> Element {
    rsx! {
        article { class: "rounded-xl border p-4 {tone.classes()}",
            span { class: "inline-flex rounded-full px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide {tone.badge()}",
                "{eyebrow}"
            }
            h2 { class: "mt-2 text-base font-semibold", "{title}" }
            p { class: "mt-1 text-xs leading-relaxed opacity-75", "{detail}" }
            if let Some(action) = action {
                div { class: "mt-3", {action} }
            }
        }
    }
}

#[component]
pub fn InlineNotice(
    title: String,
    detail: String,
    #[props(default = FindingTone::Info)] tone: FindingTone,
) -> Element {
    rsx! {
        div { class: "rounded-lg border px-3 py-2 text-sm {tone.classes()}",
            span { class: "font-medium", "{title}" }
            span { class: "ml-2 opacity-75", "{detail}" }
        }
    }
}

#[component]
pub fn LoadingPanel(label: String) -> Element {
    rsx! {
        div { class: "flex min-h-28 items-center justify-center gap-3 text-sm text-gray-500",
            span { class: "h-4 w-4 animate-spin rounded-full border-2 border-blue-600 border-t-transparent" }
            "{label}"
        }
    }
}

#[component]
pub fn UnavailablePanel(label: String, detail: String) -> Element {
    rsx! {
        div { class: "rounded-lg border border-dashed border-gray-300 bg-gray-50 px-4 py-6 text-center",
            p { class: "text-sm font-medium text-gray-700", "{label}" }
            p { class: "mt-1 text-xs text-gray-500", "{detail}" }
        }
    }
}

#[component]
pub fn ClassicLink(path: String, label: String) -> Element {
    let href = href_for(&path, UiVersion::Classic);
    rsx! {
        a {
            href,
            class: "inline-flex items-center gap-1.5 rounded-lg border border-gray-300 bg-white \
                    px-3 py-2 text-xs font-medium text-gray-700 shadow-sm hover:bg-gray-50 \
                    focus:outline-none focus:ring-2 focus:ring-blue-500 focus:ring-offset-2",
            "{label}"
            span { aria_hidden: "true", "↗" }
        }
    }
}
