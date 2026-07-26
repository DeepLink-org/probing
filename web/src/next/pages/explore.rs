use dioxus::prelude::*;
use dioxus_router::Link;

use super::super::components::{ClassicLink, NextPageHeader, SectionCard};
use super::super::routes::NextRoute;

#[component]
pub fn ExplorePage() -> Element {
    let tools = vec![
        (
            "Stacks",
            NextRoute::Stack {},
            "Live local and distributed mixed-language call stacks.",
        ),
        (
            "Rollout",
            NextRoute::Rollout {},
            "Per-trajectory phase timing across rollout workers.",
        ),
        (
            "Inference",
            NextRoute::Inference {},
            "Inference engine metrics and trends.",
        ),
        (
            "SQL Explorer",
            NextRoute::Analytics {},
            "Local and federated SQL/dataframe exploration.",
        ),
        (
            "Spans",
            NextRoute::Spans {},
            "Hierarchical Python and distributed span evidence.",
        ),
        (
            "Python Trace",
            NextRoute::Python {},
            "Function-level live variable tracing.",
        ),
        (
            "Pulsing",
            NextRoute::Pulsing {},
            "Pulsing actors, spans, metrics, and membership.",
        ),
        (
            "Process & System",
            NextRoute::System {},
            "CPU, GPU, process, thread, and environment details.",
        ),
    ];
    rsx! {
        div { class: "space-y-5",
            NextPageHeader {
                title: "Explore all capabilities".to_string(),
                subtitle: "The mature performance and runtime tools now run inside the Next shell with shared context and diagnostics.".to_string(),
            }
            div { class: "grid gap-4 md:grid-cols-2 xl:grid-cols-3",
                for (title, route, detail) in tools {
                    SectionCard {
                        title: title.to_string(),
                        subtitle: Some(detail.to_string()),
                        Link {
                            to: route,
                            class: "inline-flex rounded-lg bg-blue-600 px-3 py-2 text-xs font-medium text-white hover:bg-blue-700",
                            "Open workspace"
                        }
                    }
                }
            }
        }
    }
}

#[component]
pub fn ClassicFallbackPage(segments: Vec<String>) -> Element {
    let path = format!("/{}", segments.join("/"));
    rsx! {
        div { class: "mx-auto max-w-2xl space-y-5 py-12",
            NextPageHeader {
                title: "This tool still lives in the classic UI".to_string(),
                subtitle: format!("`{path}` is not a recognized product route. The Classic fallback remains available for compatibility."),
            }
            SectionCard {
                title: "Progressive migration boundary".to_string(),
                p { class: "text-sm leading-relaxed text-gray-600",
                    "The next interface only owns migrated diagnostic workflows. Advanced or low-level tools continue to run in the unchanged classic application."
                }
                div { class: "mt-4",
                    ClassicLink { path, label: "Open requested classic tool".to_string() }
                }
            }
        }
    }
}
