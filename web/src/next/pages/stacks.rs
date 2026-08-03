use dioxus::prelude::*;
use dioxus_router::Link;
use probing_proto::prelude::CallFrame;

use crate::api::ApiClient;
use crate::components::callstack_view::CallStackView;
use crate::components::flamegraph::{FlamegraphPayload, FlamegraphView};
use crate::state::investigation::INVESTIGATION_CONTEXT;
use crate::state::stack::{
    stack_tid_label, StackSnapshot, STACK_DIST_CLUSTER, STACK_DIST_RELOAD, STACK_MODE,
    STACK_REFRESH, STACK_SNAPSHOT,
};
use crate::utils::callframe::{
    classify_frame, count_by_kind, frame_matches_query, frame_title, matches_mode, stack_evidence,
};

use super::super::components::{
    EvidenceMetric, EvidenceSection, EvidenceSurface, FilterInput, LoadingPanel, UnavailablePanel,
    WorkspacePage,
};
use super::super::routes::NextRoute;

#[component]
pub fn StackPage() -> Element {
    let tid = INVESTIGATION_CONTEXT
        .read()
        .tid
        .map(|value| value.to_string());
    rsx! { StackWorkspace { tid } }
}

#[component]
pub fn StackThreadPage(tid: String) -> Element {
    rsx! { StackWorkspace { tid: Some(tid) } }
}

#[component]
fn StackWorkspace(tid: Option<String>) -> Element {
    let tid_label = stack_tid_label(tid.as_deref());
    let mode = STACK_MODE();
    let request_tid = tid.clone();
    let stack = use_resource(move || {
        let tid = request_tid.clone();
        let refresh = STACK_REFRESH();
        async move {
            let _ = refresh;
            ApiClient::new().get_callstack_with_mode(tid, "mixed").await
        }
    });
    let mut query = use_signal(String::new);

    rsx! {
        WorkspacePage {
            title: "Stacks".to_string(),
            subtitle: format!("One live root-to-current call path · thread {tid_label}"),
            match stack.read().clone() {
                None => rsx! { LoadingPanel { label: "Loading call stack".to_string() } },
                Some(Err(error)) => rsx! { UnavailablePanel {
                    label: "Call stack unavailable".to_string(),
                    detail: error.display_message(),
                }},
                Some(Ok(frames)) => rsx! { StackEvidenceView {
                    frames,
                    tid_label,
                    mode,
                    query: query(),
                    on_query: move |value| query.set(value),
                }},
            }
        }
    }
}

#[component]
fn StackEvidenceView(
    frames: Vec<CallFrame>,
    tid_label: String,
    mode: String,
    query: String,
    on_query: EventHandler<String>,
) -> Element {
    let evidence = stack_evidence(&frames);
    let filtered = frames
        .iter()
        .enumerate()
        .filter(|(_, frame)| matches_mode(frame, &mode))
        .filter(|(_, frame)| frame_matches_query(frame, &query))
        .map(|(index, frame)| (index, frame.clone()))
        .collect::<Vec<_>>();
    let shown = filtered.len();
    let total = frames.len();
    let (py, rust, cpp) = count_by_kind(&frames);
    let snapshot = StackSnapshot {
        tid_label: tid_label.clone(),
        total,
        py,
        rust,
        cpp,
        shown,
        loaded: true,
    };
    use_effect(use_reactive!(|(snapshot,)| {
        *STACK_SNAPSHOT.write() = snapshot;
    }));

    rsx! {
        EvidenceSurface {
            EvidenceSection {
                title: "Captured evidence".to_string(),
                subtitle: Some("Counts describe this on-demand sample; they do not infer a bottleneck.".to_string()),
                div { class: "grid grid-cols-5 divide-x divide-gray-200",
                    EvidenceMetric { label: "Frames", value: total.to_string() }
                    EvidenceMetric { label: "Python", value: evidence.python.to_string() }
                    EvidenceMetric { label: "Rust", value: evidence.rust.to_string() }
                    EvidenceMetric { label: "Native", value: evidence.native.to_string() }
                    EvidenceMetric { label: "Runtime boundaries", value: evidence.runtime_boundaries.to_string() }
                }
                if !frames.is_empty() {
                    div { class: "mt-4 flex h-5 gap-1 overflow-x-auto", title: "Root to current frame",
                        for frame in &frames {
                            span {
                                class: "min-w-2 flex-1 rounded-sm {classify_frame(frame).overview_cell()}",
                                title: "{frame_title(frame)}",
                            }
                        }
                    }
                    p { class: "mt-1 text-xs text-gray-500", "Root → current · one segment per frame" }
                }
            }
            EvidenceSection {
                title: "Call hierarchy".to_string(),
                subtitle: Some(format!("{shown} of {total} frames · expand only the frame details you need.")),
                divided: true,
                actions: Some(rsx! {
                    FilterInput {
                        value: query,
                        placeholder: "Find function or source file".to_string(),
                        oninput: move |value| on_query.call(value),
                    }
                }),
                if filtered.is_empty() {
                    UnavailablePanel {
                        label: "No matching stack frames".to_string(),
                        detail: if frames.is_empty() {
                            "The thread may be idle or not yet sampled.".to_string()
                        } else {
                            "Change the language mode or search text.".to_string()
                        },
                    }
                } else {
                    for (visible, (index, frame)) in filtered.iter().enumerate() {
                        CallStackView {
                            key: "{index}-{frame_title(frame)}",
                            callstack: frame.clone(),
                            index: *index,
                            is_last: visible + 1 == shown,
                            default_open: *index + 1 == total,
                            position: frame_position(*index, total),
                        }
                    }
                }
            }
        }
    }
}

fn frame_position(index: usize, total: usize) -> Option<String> {
    match (index == 0, index + 1 == total) {
        (true, true) => Some("root · current".to_string()),
        (true, false) => Some("root".to_string()),
        (false, true) => Some("current".to_string()),
        (false, false) => None,
    }
}

#[component]
pub fn DistributedStackPage() -> Element {
    rsx! { DistributedStackWorkspace { mode: "mixed".to_string() } }
}

#[component]
pub fn DistributedPythonStackPage() -> Element {
    rsx! { DistributedStackWorkspace { mode: "py".to_string() } }
}

#[component]
fn DistributedStackWorkspace(mode: String) -> Element {
    let request_mode = mode.clone();
    let profile = use_resource(move || {
        let mode = request_mode.clone();
        let cluster = *STACK_DIST_CLUSTER.read();
        let reload = *STACK_DIST_RELOAD.read();
        async move {
            let _ = reload;
            let body = ApiClient::new()
                .get_distributed_stack_flamegraph_json(cluster, &mode)
                .await?;
            serde_json::from_str::<FlamegraphPayload>(&body).map_err(|error| {
                crate::utils::error::AppError::Api(format!("Invalid flamegraph JSON: {error}"))
            })
        }
    });
    let cluster = *STACK_DIST_CLUSTER.read();
    let scope = if cluster {
        "cluster fan-out"
    } else {
        "this node"
    };

    rsx! {
        WorkspacePage {
            title: "Distributed stacks".to_string(),
            subtitle: format!("Identical {} call paths merged by sample count · {scope}.", if mode == "py" { "Python" } else { "mixed-language" }),
            fill: true,
            actions: Some(rsx! {
                    Link { to: NextRoute::DistributedStack {}, class: stack_tab(mode == "mixed"), "Full stack" }
                    Link { to: NextRoute::DistributedPythonStack {}, class: stack_tab(mode == "py"), "Python only" }
                }),
            match profile.read().clone() {
                None => rsx! { LoadingPanel { label: "Loading distributed stack samples".to_string() } },
                Some(Err(error)) => rsx! { UnavailablePanel {
                    label: "Distributed stack samples unavailable".to_string(),
                    detail: error.display_message(),
                }},
                Some(Ok(payload)) => rsx! { DistributedProfile { payload } },
            }
        }
    }
}

fn stack_tab(active: bool) -> &'static str {
    if active {
        "rounded-lg bg-blue-600 px-3 py-1.5 text-xs font-medium text-white"
    } else {
        "rounded-lg border border-gray-300 bg-white px-3 py-1.5 text-xs text-gray-700"
    }
}

#[component]
fn DistributedProfile(payload: FlamegraphPayload) -> Element {
    let failed = payload.nodes_failed.len();
    let failed_names = payload.nodes_failed.join(", ");
    rsx! {
        div { class: "min-h-0 flex-1",
            EvidenceSurface { fill: true,
                EvidenceSection {
                    title: "Returned evidence".to_string(),
                    subtitle: Some("Sample and peer counts expose result coverage before inspection.".to_string()),
                    div { class: "grid grid-cols-4 divide-x divide-gray-200",
                        EvidenceMetric { label: "Samples", value: payload.total.to_string() }
                        EvidenceMetric { label: "Ranks included", value: payload.rank_count.map(|v| v.to_string()).unwrap_or_else(|| "—".to_string()) }
                        EvidenceMetric { label: "Merged frames", value: payload.frames.len().to_string() }
                        EvidenceMetric { label: "Failed peers", value: failed.to_string() }
                    }
                    if failed > 0 {
                        details { class: "mt-2 rounded-md border border-amber-200 bg-amber-50 px-3 py-2 text-xs text-amber-900",
                            summary {
                                class: "cursor-pointer font-medium focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-amber-600 focus-visible:ring-offset-2",
                                "Partial result · {failed} peer(s) did not contribute · show peers"
                            }
                            p { class: "mt-1 break-all font-mono text-xs", "{failed_names}" }
                        }
                    }
                }
                div { class: "min-h-0 flex-1 overflow-hidden border-t border-gray-200",
                    FlamegraphView { payload, thread_tid: None, torch_metric: None, on_torch_metric: None }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_positions_preserve_root_and_current() {
        assert_eq!(frame_position(0, 3).as_deref(), Some("root"));
        assert_eq!(frame_position(2, 3).as_deref(), Some("current"));
        assert_eq!(frame_position(0, 1).as_deref(), Some("root · current"));
    }
}
