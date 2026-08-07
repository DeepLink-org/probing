use dioxus::prelude::*;

use crate::state::commands::COMMAND_PANEL_OPEN;
use crate::state::investigation::INVESTIGATION_CONTEXT;

use super::super::components::{
    evidence_href, ActionButton, FilterInput, SectionCard, WorkspacePage,
};
use super::super::routes::NextRoute;

#[derive(Clone, Debug, PartialEq, Eq)]
struct CapabilityItem {
    title: String,
    group: String,
    description: String,
    path: String,
    href: String,
}

#[component]
pub fn ExplorePage() -> Element {
    let mut filter = use_signal(String::new);
    let context = INVESTIGATION_CONTEXT.read().clone();
    let items = capability_items(&filter(), &context);
    let result_count = items.len();
    rsx! {
        WorkspacePage {
            title: "Capability Catalog".to_string(),
            subtitle: "Search canonical Next workspaces by their registered title, group, path, or evidence description.".to_string(),
            actions: rsx! {
                ActionButton { label: "Open command palette".to_string(), compact: true, onclick: move |_| *COMMAND_PANEL_OPEN.write() = true }
            },
            SectionCard {
                title: "Workspaces".to_string(),
                subtitle: Some(format!("{result_count} matching canonical routes; navigation preserves the pinned investigation context.")),
                body_class: "p-0".to_string(),
                div { class: "border-b border-gray-100 p-3",
                    FilterInput {
                        class: "w-full".to_string(),
                        value: filter(),
                        placeholder: "Filter title, group, path, or evidence".to_string(),
                        oninput: move |value| filter.set(value),
                    }
                }
                if items.is_empty() {
                    div { class: "px-4 py-8 text-center text-xs text-gray-500", "No canonical workspace matches this filter." }
                } else {
                    div { class: "divide-y divide-gray-100",
                        for item in items {
                            a {
                                href: item.href,
                                class: "grid grid-cols-[minmax(180px,0.7fr)_minmax(0,1.7fr)_auto] items-center gap-4 px-4 py-3 hover:bg-blue-50/60 focus:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-blue-600",
                                div { class: "min-w-0",
                                    div { class: "truncate text-sm font-medium text-gray-900", "{item.title}" }
                                    div { class: "mt-0.5 text-xs text-gray-500", "{item.group}" }
                                }
                                p { class: "min-w-0 text-xs leading-relaxed text-gray-600", "{item.description}" }
                                div { class: "text-right",
                                    div { class: "font-mono text-xs text-gray-500", "{item.path}" }
                                    div { class: "mt-0.5 text-xs font-medium text-blue-700", "Open →" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn capability_items(
    filter: &str,
    context: &crate::state::investigation::InvestigationContext,
) -> Vec<CapabilityItem> {
    let needle = filter.trim().to_ascii_lowercase();
    canonical_capability_routes()
        .into_iter()
        .filter_map(|route| {
            let spec = route.page_spec();
            if !matches_catalog_filter(&spec, &needle) {
                return None;
            }
            Some(CapabilityItem {
                title: spec.title.to_string(),
                group: spec.sidebar_group.to_string(),
                description: spec.description.to_string(),
                path: spec.canonical_path.to_string(),
                href: evidence_href(&route, context),
            })
        })
        .collect()
}

fn matches_catalog_filter(spec: &super::super::page_registry::PageSpec, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    format!(
        "{} {} {} {}",
        spec.title, spec.sidebar_group, spec.canonical_path, spec.description
    )
    .to_ascii_lowercase()
    .contains(needle)
}

fn canonical_capability_routes() -> Vec<NextRoute> {
    vec![
        NextRoute::Dashboard {},
        NextRoute::Investigate {},
        NextRoute::Distributed {},
        NextRoute::Cluster {},
        NextRoute::DistributedStatus {},
        NextRoute::Training {},
        NextRoute::Inference {},
        NextRoute::Rollout {},
        NextRoute::Memory {},
        NextRoute::Profiles {},
        NextRoute::Stack {},
        NextRoute::Spans {},
        NextRoute::Analytics {},
        NextRoute::Python {},
        NextRoute::Pulsing {},
        NextRoute::System {},
    ]
}

#[component]
pub fn NotFoundPage(segments: Vec<String>) -> Element {
    let path = format!("/{}", segments.join("/"));
    let explore_href = evidence_href(
        &NextRoute::Explore {},
        &INVESTIGATION_CONTEXT.read().clone(),
    );
    rsx! {
        div { class: "mx-auto max-w-2xl py-12",
            WorkspacePage {
                title: "Route not found".to_string(),
                subtitle: format!("{path} is not a registered product route."),
                SectionCard {
                    title: "Available workspaces".to_string(),
                    subtitle: Some("Open the capability catalog to find the matching evidence surface.".to_string()),
                    a { href: explore_href, class: "inline-flex rounded-lg border border-gray-300 bg-white px-3 py-2 text-xs font-medium text-gray-700 hover:bg-gray-50", "Browse workspaces" }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_uses_registered_metadata_and_filters_paths() {
        let memory = NextRoute::Memory {}.page_spec();
        assert_eq!(memory.title, "Memory");
        assert!(matches_catalog_filter(&memory, "/memory"));
        assert!(!matches_catalog_filter(&memory, "/training"));
    }

    #[test]
    fn catalog_contains_only_canonical_routes() {
        let routes = canonical_capability_routes();
        assert!(routes.iter().all(|route| {
            route.page_spec().canonical_path != "/explore"
                && !matches!(route, NextRoute::NotFound { .. })
        }));
    }
}
