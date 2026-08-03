use dioxus::prelude::*;
use dioxus_router::Link;

use super::super::components::{ClassicLink, SectionCard, WorkspacePage};
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
        WorkspacePage {
            title: "Explore all capabilities".to_string(),
            subtitle: "The mature performance and runtime tools now run inside the Next shell with shared context and diagnostics.".to_string(),
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
        div { class: "mx-auto max-w-2xl py-12",
            WorkspacePage {
                title: "Unknown Next route".to_string(),
                subtitle: format!("`{path}` is not a documented product route. The Classic fallback remains available for historical or private paths."),
                SectionCard {
                    title: "Compatibility boundary".to_string(),
                    p { class: "text-sm leading-relaxed text-gray-600",
                        "All documented product routes have native Next pages. This unrecognized path can still be opened in the frozen Classic application while compatibility is retained."
                    }
                    div { class: "mt-4",
                        ClassicLink { path, label: "Open requested classic tool".to_string() }
                    }
                }
            }
        }
    }
}
