use dioxus::prelude::*;
use dioxus_router::use_route;

use crate::components::icon::Icon;
use crate::state::investigation::{clear_investigation_context, INVESTIGATION_CONTEXT};
use crate::state::investigation_url::context_to_search;
use crate::utils::base_path::with_base;

use super::routes::NextRoute;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ActionTone {
    Primary,
    #[default]
    Neutral,
    Danger,
}

impl ActionTone {
    fn classes(self) -> &'static str {
        match self {
            Self::Primary => "border-blue-600 bg-blue-600 text-white hover:bg-blue-700",
            Self::Neutral => "border-gray-300 bg-white text-gray-700 hover:bg-gray-50",
            Self::Danger => "border-red-200 bg-red-50 text-red-700 hover:bg-red-100",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeTone {
    Warning,
    Info,
}

impl NoticeTone {
    fn classes(self) -> &'static str {
        match self {
            Self::Warning => "border-amber-200 bg-amber-50 text-amber-950",
            Self::Info => "border-blue-200 bg-blue-50 text-blue-950",
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
                h1 { class: "text-xl font-semibold tracking-tight text-gray-950", "{title}" }
                p { class: "mt-0.5 max-w-3xl text-xs text-gray-500", "{subtitle}" }
            }
            if let Some(actions) = actions {
                div { class: "flex shrink-0 flex-wrap items-center gap-2", {actions} }
            }
        }
    }
}

/// Canonical page frame for the Next shell. Page content owns evidence only;
/// title spacing, direct actions, and full-height behavior stay consistent.
#[component]
pub fn WorkspacePage(
    title: String,
    subtitle: String,
    children: Element,
    #[props(optional)] actions: Option<Element>,
    #[props(default = false)] fill: bool,
) -> Element {
    let route = use_route::<NextRoute>();
    let class = if fill {
        "flex h-full min-h-0 flex-col gap-4"
    } else {
        "space-y-4"
    };
    rsx! {
        div { class,
            NextPageHeader { title, subtitle, actions }
            InvestigationBar { support: route.investigation_support() }
            {children}
        }
    }
}

/// Persistent, URL-backed evidence selection shared by diagnostic pages.
/// It stays deliberately flat so context is visible without becoming another card.
#[component]
pub fn InvestigationBar(support: super::page_registry::InvestigationSupport) -> Element {
    let context = INVESTIGATION_CONTEXT.read().clone();
    if context.is_empty() {
        return rsx! {};
    }

    let stack_route = context
        .tid
        .map(|tid| NextRoute::StackThread {
            tid: tid.to_string(),
        })
        .unwrap_or(NextRoute::Stack {});
    let unused_fields = unused_context_fields(&context, support);
    let unused_label = unused_fields.join(", ");

    rsx! {
        div {
            class: "flex min-h-9 flex-wrap items-center gap-x-3 gap-y-1 border-y border-blue-100 bg-blue-50/70 px-3 py-1.5 text-xs",
            aria_live: "polite",
            div { class: "flex min-w-0 flex-1 flex-wrap items-center gap-1.5",
                span { class: "mr-1 font-semibold uppercase tracking-wide text-blue-700",
                    "Pinned context"
                }
                if let Some(pid) = context.pid {
                    ContextChip { label: format!("pid {pid}"), available_here: support.pid }
                }
                if let Some(step) = context.local_step {
                    ContextChip { label: format!("step {step}"), available_here: support.step }
                }
                if let Some(rank) = context.rank {
                    ContextChip { label: format!("rank {rank}"), available_here: support.rank }
                }
                if let Some(ref host) = context.host {
                    ContextChip { label: host.clone(), available_here: support.host }
                }
                if let Some(device_id) = context.device_id {
                    ContextChip { label: format!("GPU {device_id}"), available_here: support.device }
                }
                if let Some(trace_id) = context.trace_id {
                    ContextChip { label: format!("trace {trace_id}"), available_here: support.trace }
                }
                if let Some(tid) = context.tid {
                    ContextChip { label: format!("thread {tid}"), available_here: support.tid }
                }
                if let Some(span) = context.span_name {
                    ContextChip { label: span, available_here: support.span }
                }
                if !unused_fields.is_empty() {
                    span { class: "ml-1 text-gray-500",
                        "Not used here: {unused_label}"
                    }
                }
            }
            nav { class: "flex shrink-0 items-center gap-1", aria_label: "Continue investigation",
                if context.rank.is_some() || context.local_step.is_some() {
                    ContextLink { route: NextRoute::Training {}, label: "Training" }
                }
                if context.rank.is_some() || context.host.is_some() || context.device_id.is_some() {
                    ContextLink { route: NextRoute::Memory {}, label: "Memory" }
                }
                ContextLink { route: NextRoute::Spans {}, label: "Tracing" }
                ContextLink { route: stack_route, label: "Stacks" }
                ContextLink { route: NextRoute::Profiles {}, label: "Profiling" }
                ContextLink { route: NextRoute::Investigate {}, label: "Investigate" }
                button {
                    r#type: "button",
                    class: "ml-1 rounded px-1.5 py-1 text-gray-600 hover:bg-white hover:text-gray-900 focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-600",
                    aria_label: "Clear pinned investigation context",
                    onclick: move |_| clear_investigation_context(),
                    "Clear"
                }
            }
        }
    }
}

#[component]
fn ContextChip(label: String, available_here: bool) -> Element {
    let class = if available_here {
        "max-w-80 break-all rounded bg-white px-1.5 py-0.5 font-mono leading-5 text-blue-900 ring-1 ring-blue-100"
    } else {
        "max-w-80 break-all rounded bg-gray-50 px-1.5 py-0.5 font-mono leading-5 text-gray-500 ring-1 ring-gray-200"
    };
    rsx! {
        span {
            class,
            title: if available_here { "This page has evidence that can use this coordinate" } else { "This page does not use this coordinate" },
            "{label}"
        }
    }
}

fn unused_context_fields(
    context: &crate::state::investigation::InvestigationContext,
    support: super::page_registry::InvestigationSupport,
) -> Vec<&'static str> {
    let mut fields = Vec::new();
    if context.pid.is_some() && !support.pid {
        fields.push("pid");
    }
    if context.tid.is_some() && !support.tid {
        fields.push("thread");
    }
    if context.rank.is_some() && !support.rank {
        fields.push("rank");
    }
    if context.host.is_some() && !support.host {
        fields.push("host");
    }
    if context.device_id.is_some() && !support.device {
        fields.push("GPU");
    }
    if context.trace_id.is_some() && !support.trace {
        fields.push("trace");
    }
    if context.span_name.is_some() && !support.span {
        fields.push("span");
    }
    if context.local_step.is_some() && !support.step {
        fields.push("step");
    }
    fields
}

#[component]
fn ContextLink(route: NextRoute, label: &'static str) -> Element {
    rsx! {
        EvidenceLink {
            route,
            label: label.to_string(),
            class_name: "rounded px-1.5 py-1 font-medium text-blue-700 hover:bg-white hover:text-blue-900 focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-600".to_string(),
        }
    }
}

/// A durable cross-page evidence link. The rendered href includes the complete
/// investigation coordinates, so regular clicks, copied links, and new tabs
/// all continue from the same evidence selection.
#[component]
pub fn EvidenceLink(
    route: NextRoute,
    label: String,
    #[props(default = String::from("text-xs font-medium text-blue-600 hover:underline"))]
    class_name: String,
) -> Element {
    let context = INVESTIGATION_CONTEXT.read().clone();
    let href = evidence_href(&route, &context);
    rsx! {
        a { href, class: "{class_name}", "{label}" }
    }
}

pub fn evidence_href(
    route: &NextRoute,
    context: &crate::state::investigation::InvestigationContext,
) -> String {
    let path = with_base(&route.to_string());
    append_evidence_context(path, context)
}

fn append_evidence_context(
    mut href: String,
    context: &crate::state::investigation::InvestigationContext,
) -> String {
    let query = context_to_search(context);
    if query.is_empty() {
        return href;
    }
    if href.contains('?') {
        href.push('&');
    } else {
        href.push('?');
    }
    href.push_str(&query);
    href
}

/// A single page-level evidence plane. Related sections should use dividers
/// inside this surface instead of becoming a stack of equal-weight cards.
#[component]
pub fn EvidenceSurface(children: Element, #[props(default = false)] fill: bool) -> Element {
    let class = if fill {
        "flex h-full min-h-0 flex-col overflow-hidden rounded-lg border border-gray-200 bg-white"
    } else {
        "overflow-hidden rounded-lg border border-gray-200 bg-white"
    };
    rsx! { section { class: "{class}", {children} } }
}

#[component]
pub fn EvidenceSection(
    title: String,
    children: Element,
    #[props(optional)] subtitle: Option<String>,
    #[props(optional)] actions: Option<Element>,
    #[props(default = String::from("p-4"))] body_class: String,
    #[props(default = false)] divided: bool,
) -> Element {
    rsx! {
        section { class: if divided { "border-t border-gray-200" } else { "" },
            div { class: "flex flex-wrap items-start justify-between gap-3 px-4 pb-2 pt-3",
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
pub fn SectionCard(
    title: String,
    children: Element,
    #[props(optional)] subtitle: Option<String>,
    #[props(optional)] actions: Option<Element>,
    #[props(default = String::from("p-4"))] body_class: String,
    #[props(default = false)] fill: bool,
) -> Element {
    let section_class = if fill {
        "flex h-full min-h-0 flex-col overflow-hidden rounded-lg border border-gray-200 bg-white"
    } else {
        "overflow-hidden rounded-lg border border-gray-200 bg-white"
    };
    rsx! {
        section { class: "{section_class}",
            div { class: "flex flex-wrap items-start justify-between gap-3 border-b border-gray-100 px-4 py-2.5",
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
        div { class: "rounded-lg border border-gray-200 bg-white px-3.5 py-2.5",
            div { class: "flex items-center gap-2 text-xs font-medium uppercase tracking-wide text-gray-500",
                if let Some(icon) = icon {
                    Icon { icon, class: "h-4 w-4 text-blue-600" }
                }
                "{label}"
            }
            div { class: "mt-1.5 text-xl font-semibold tabular-nums text-gray-950", "{value}" }
            if let Some(detail) = detail {
                div { class: "mt-1 text-xs text-gray-500", "{detail}" }
            }
        }
    }
}

/// Compact factual metric used inside a card-level evidence strip.
#[component]
pub fn EvidenceMetric(
    label: String,
    value: String,
    #[props(optional)] detail: Option<String>,
) -> Element {
    rsx! {
        div { class: "min-w-0 px-3 first:pl-0 last:pr-0",
            div { class: "text-xs font-medium uppercase tracking-wide text-gray-600", "{label}" }
            div { class: "mt-1 truncate text-lg font-semibold tabular-nums text-gray-950", "{value}" }
            if let Some(detail) = detail {
                div { class: "mt-0.5 break-words text-xs leading-4 text-gray-500", "{detail}" }
            }
        }
    }
}

/// Standard direct-manipulation search field. Navigation and configuration
/// remain in the sidebar; this input only filters evidence already on the page.
#[component]
pub fn FilterInput(
    value: String,
    placeholder: String,
    oninput: EventHandler<String>,
    #[props(default = String::from("w-64"))] class: String,
) -> Element {
    rsx! {
        input {
            r#type: "search",
            class: "{class} rounded-lg border border-gray-300 bg-white px-3 py-1.5 text-xs text-gray-800 outline-none focus:border-blue-500 focus:ring-2 focus:ring-blue-500/20",
            placeholder: placeholder.clone(),
            aria_label: "{placeholder}",
            value,
            oninput: move |event| oninput.call(event.value()),
        }
    }
}

/// Standard text action for page-local operations such as run, export, or
/// expand. It is deliberately not used for route navigation.
#[component]
pub fn ActionButton(
    label: String,
    onclick: EventHandler<()>,
    #[props(default)] tone: ActionTone,
    #[props(default = false)] disabled: bool,
    #[props(default = false)] compact: bool,
) -> Element {
    let tone_class = tone.classes();
    let size_class = if compact {
        "rounded-md px-2 py-1 text-xs"
    } else {
        "rounded-lg px-3 py-1.5 text-xs"
    };
    rsx! {
        button {
            r#type: "button",
            class: "border font-medium shadow-sm focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-600 focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50 {size_class} {tone_class}",
            disabled,
            onclick: move |_| onclick.call(()),
            "{label}"
        }
    }
}

#[component]
pub fn InlineNotice(
    title: String,
    detail: String,
    #[props(default = NoticeTone::Info)] tone: NoticeTone,
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
        div { class: "flex min-h-20 items-center justify-center gap-3 text-sm text-gray-600", role: "status", aria_live: "polite",
            span { class: "h-4 w-4 animate-spin rounded-full border-2 border-blue-600 border-t-transparent motion-reduce:animate-none", aria_hidden: "true" }
            "{label}"
        }
    }
}

#[component]
pub fn UnavailablePanel(label: String, detail: String) -> Element {
    rsx! {
        div { class: "rounded-md border border-dashed border-gray-300 bg-gray-50 px-4 py-3 text-left",
            p { class: "text-sm font-medium text-gray-700", "{label}" }
            p { class: "mt-1 text-xs text-gray-500", "{detail}" }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::investigation::InvestigationContext;

    #[test]
    fn evidence_links_are_shareable_with_the_same_coordinates() {
        let context = InvestigationContext {
            rank: Some(7),
            device_id: Some(3),
            local_step: Some(31),
            ..Default::default()
        };
        let href = append_evidence_context("/memory".to_string(), &context);

        assert!(href.starts_with("/memory?"));
        assert!(href.contains("rank=7"));
        assert!(href.contains("gpu=3"));
        assert!(href.contains("step=31"));
        assert!(!href.contains("ui="));
    }

    #[test]
    fn unused_context_fields_are_named_instead_of_counted() {
        let context = InvestigationContext {
            rank: Some(7),
            host: Some("node-1".to_string()),
            device_id: Some(3),
            local_step: Some(31),
            span_name: Some("train.step".to_string()),
            ..Default::default()
        };
        let support = super::super::page_registry::InvestigationSupport {
            rank: true,
            device: true,
            ..Default::default()
        };

        assert_eq!(
            unused_context_fields(&context, support),
            vec!["host", "span", "step"]
        );
    }
}
