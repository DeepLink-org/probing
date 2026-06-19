pub mod diff;
pub mod explorer;
pub mod logic;
pub mod model;
pub mod widgets;

use dioxus::prelude::*;

use crate::components::common::EmptyState;

use explorer::StackExplorerView;
use widgets::{FlamegraphShell, FlamegraphToolbar, MetricPill, MetricPillRow};

use logic::TORCH_METRICS;

pub use model::FlamegraphPayload;

#[component]
pub fn FlamegraphView(
    payload: FlamegraphPayload,
    #[props(optional)] torch_metric: Option<Signal<String>>,
    #[props(optional)] on_torch_metric: Option<EventHandler<String>>,
    #[props(optional)] thread_tid: Option<i32>,
) -> Element {
    if payload.frames.is_empty() {
        let message = payload
            .empty_message
            .clone()
            .unwrap_or_else(|| "No profiling samples collected".to_string());
        let metric = torch_metric
            .map(|s| s())
            .or(payload.metric.clone())
            .unwrap_or_else(|| "duration".to_string());
        if let Some(on_metric) = on_torch_metric {
            return rsx! {
                FlamegraphShell {
                    FlamegraphToolbar {
                        MetricPillRow {
                        for (id, label) in TORCH_METRICS.iter() {
                            {
                                let id = id.to_string();
                                let label = label.to_string();
                                let active = metric == id;
                                rsx! {
                                    MetricPill {
                                        label,
                                        active,
                                        onclick: EventHandler::new(move |_| on_metric.call(id.clone())),
                                    }
                                }
                            }
                        }
                        }
                    }
                    EmptyState { message }
                }
            };
        }
        return rsx! {
            EmptyState { message }
        };
    }

    match payload.profile.as_str() {
        "classic" => rsx! {
            EmptyState {
                message: "Unsupported flamegraph profile. Reload the page.".to_string(),
            }
        },
        _ => {
            let dropped = payload.dropped;
            rsx! {
                if dropped > 0 {
                    div {
                        class: "px-4 py-2 text-xs text-amber-800 bg-amber-50 border-b border-amber-200",
                        "Warning: {dropped} samples dropped (ring full or cardinality cap); results may undercount. Lower the sampling frequency or let the buffer drain."
                    }
                }
                StackExplorerView {
                    payload,
                    torch_metric,
                    on_torch_metric,
                    thread_tid,
                }
            }
        }
    }
}
