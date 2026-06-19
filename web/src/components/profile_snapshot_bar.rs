use dioxus::prelude::*;

use crate::components::flamegraph::diff::{compute_frame_deltas, FrameDelta};
use crate::components::flamegraph::FlamegraphPayload;
use crate::state::profile_snapshots::{
    push_profile_snapshot, snapshot_label, ProfileSnapshot, PROFILE_DIFF_BASELINE,
    PROFILE_SNAPSHOTS,
};

#[component]
pub fn ProfileSnapshotBar(
    profiler: String,
    metric: Option<String>,
    payload: FlamegraphPayload,
) -> Element {
    let mut auto_captured = use_signal(|| false);
    use_effect({
        let payload = payload.clone();
        let profiler = profiler.clone();
        let metric = metric.clone();
        move || {
            if !auto_captured() && !payload.frames.is_empty() {
                push_profile_snapshot(&profiler, metric.as_deref(), payload.clone());
                auto_captured.set(true);
            }
        }
    });
    let snapshots: Vec<ProfileSnapshot> = PROFILE_SNAPSHOTS
        .read()
        .iter()
        .filter(|s| s.profiler == profiler)
        .cloned()
        .collect();
    let baseline_id = *PROFILE_DIFF_BASELINE.read();
    let baseline = baseline_id.and_then(|id| {
        PROFILE_SNAPSHOTS
            .read()
            .iter()
            .find(|s| s.id == id)
            .cloned()
    });

    let diffs = baseline
        .as_ref()
        .map(|base| compute_frame_deltas(&payload, &base.payload));

    rsx! {
        div { class: "border-b border-gray-200 bg-gray-50/80 px-4 py-3 space-y-3",
            div { class: "flex flex-wrap items-center gap-2",
                span { class: "text-xs font-semibold text-gray-600 uppercase tracking-wide", "Snapshots" }
                button {
                    class: "px-2.5 py-1 text-xs rounded-md border border-gray-300 bg-white hover:bg-gray-100 text-gray-700",
                    onclick: {
                        let payload = payload.clone();
                        let metric_ref = metric.clone();
                        move |_| {
                            push_profile_snapshot(
                                &profiler,
                                metric_ref.as_deref(),
                                payload.clone(),
                            );
                        }
                    },
                    "Capture"
                }
                if snapshots.is_empty() {
                    span { class: "text-xs text-gray-400", "Auto-captured on load · compare regressions" }
                } else {
                    for snap in snapshots {
                        {
                            let id = snap.id;
                            let is_baseline = baseline_id == Some(id);
                            rsx! {
                                button {
                                    class: if is_baseline {
                                        "px-2.5 py-1 text-xs rounded-md border border-emerald-300 bg-emerald-50 text-emerald-800 font-medium"
                                    } else {
                                        "px-2.5 py-1 text-xs rounded-md border border-gray-300 bg-white hover:bg-gray-100 text-gray-700"
                                    },
                                    title: "Use as diff baseline",
                                    onclick: move |_| {
                                        if is_baseline {
                                            *PROFILE_DIFF_BASELINE.write() = None;
                                        } else {
                                            *PROFILE_DIFF_BASELINE.write() = Some(id);
                                        }
                                    },
                                    "{snapshot_label(&snap)}"
                                }
                            }
                        }
                    }
                    if baseline_id.is_some() {
                        button {
                            class: "px-2 py-1 text-xs rounded-md text-gray-500 hover:text-gray-800",
                            onclick: move |_| *PROFILE_DIFF_BASELINE.write() = None,
                            "Clear diff"
                        }
                    }
                }
            }
            if let Some(rows) = diffs {
                if !rows.is_empty() {
                    ProfileDiffTable { rows }
                }
            }
        }
    }
}

#[component]
fn ProfileDiffTable(rows: Vec<FrameDelta>) -> Element {
    rsx! {
        div { class: "rounded-lg border border-gray-200 overflow-hidden",
            div { class: "px-3 py-2 bg-white border-b border-gray-100 text-xs font-medium text-gray-700",
                "Top changes vs baseline"
            }
            div { class: "max-h-40 overflow-y-auto",
                table { class: "w-full text-xs",
                    thead {
                        tr { class: "bg-gray-50 text-gray-500 text-left",
                            th { class: "px-3 py-1.5 font-medium", "Frame" }
                            th { class: "px-3 py-1.5 font-medium text-right", "Baseline" }
                            th { class: "px-3 py-1.5 font-medium text-right", "Current" }
                            th { class: "px-3 py-1.5 font-medium text-right", "Δ" }
                        }
                    }
                    tbody {
                        for row in rows {
                            tr { class: "border-t border-gray-100",
                                td { class: "px-3 py-1.5 font-mono text-gray-800 truncate max-w-[16rem]", "{row.path}" }
                                td { class: "px-3 py-1.5 text-right font-mono text-gray-500", "{row.baseline}" }
                                td { class: "px-3 py-1.5 text-right font-mono text-gray-800", "{row.current}" }
                                td {
                                    class: if row.delta > 0 {
                                        "px-3 py-1.5 text-right font-mono text-red-600 font-medium"
                                    } else {
                                        "px-3 py-1.5 text-right font-mono text-emerald-600 font-medium"
                                    },
                                    if row.delta > 0 { "+{row.delta}" } else { "{row.delta}" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
