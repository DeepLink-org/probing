use dioxus::prelude::*;
use dioxus_router::use_navigator;
use probing_proto::prelude::{RlRunSummary, RlSampleSummary};

use crate::api::ApiClient;
use crate::hooks::{use_page_visible, use_polled_resource};
use crate::state::rl::{ROLLOUT_FILTER, ROLLOUT_FILTER_INPUT, SAMPLE_STEP_FILTER};
use crate::utils::error::Result;

use super::super::components::{LoadingPanel, UnavailablePanel, WorkspacePage};
use super::super::rl_run::{read_selected_run_id, resolve_run, write_selected_run_id};
use super::super::routes::NextRoute;

const POLL_MS: u32 = 5_000;
const SAMPLE_LIMIT: usize = 500;

#[derive(Clone, Debug, PartialEq)]
struct SamplesEvidence {
    runs: Vec<RlRunSummary>,
    run_id: String,
    samples: Vec<RlSampleSummary>,
}

#[component]
pub fn RlSamplesPage() -> Element {
    let visible = use_page_visible();
    let mut selected_run_id = use_signal(|| read_selected_run_id().unwrap_or_default());
    let evidence = use_polled_resource(POLL_MS, Some(visible), move || {
        let preferred = selected_run_id();
        async move { load_samples(preferred).await }
    });
    let state = evidence.read().clone();

    rsx! {
        WorkspacePage {
            title: "RL Samples".to_string(),
            subtitle: "Recent rollout outcomes with direct links into the distributed span hierarchy.".to_string(),
            actions: rsx! { span { class: "text-xs text-gray-500", "Live · {POLL_MS / 1000}s" } },
            match state {
                None => rsx! { LoadingPanel { label: "Loading RL samples".to_string() } },
                Some(Err(error)) => rsx! { UnavailablePanel {
                    label: "RL samples unavailable".to_string(),
                    detail: error.display_message(),
                }},
                Some(Ok(evidence)) if evidence.samples.is_empty() => rsx! { UnavailablePanel {
                    label: "No RL samples reported".to_string(),
                    detail: "XTuner samples appear after a trajectory batch is persisted.".to_string(),
                }},
                Some(Ok(evidence)) => rsx! {
                    SamplesTable {
                        evidence,
                        selected_run_id,
                        on_select_run: move |run_id: String| {
                            write_selected_run_id(&run_id);
                            selected_run_id.set(run_id);
                        },
                    }
                },
            }
        }
    }
}

async fn load_samples(preferred: String) -> Result<SamplesEvidence> {
    let client = ApiClient::new();
    let runs = client.fetch_rl_runs().await?.runs;
    let Some(run) = resolve_run(&runs, Some(preferred.as_str())).cloned() else {
        return Ok(SamplesEvidence {
            runs,
            run_id: String::new(),
            samples: Vec::new(),
        });
    };
    let run_id = run.run_id.clone();
    let samples = client
        .fetch_rl_samples(&run_id, SAMPLE_LIMIT)
        .await?
        .samples;
    Ok(SamplesEvidence {
        runs,
        run_id,
        samples,
    })
}

#[component]
fn SamplesTable(
    evidence: SamplesEvidence,
    selected_run_id: Signal<String>,
    on_select_run: EventHandler<String>,
) -> Element {
    let navigator = use_navigator();
    let current = selected_run_id();
    let step_filter = SAMPLE_STEP_FILTER();
    let visible_samples = evidence
        .samples
        .iter()
        .filter(|sample| step_filter.map(|step| sample.step == step).unwrap_or(true))
        .cloned()
        .collect::<Vec<_>>();
    rsx! {
        div { class: "mb-3 flex flex-wrap items-center justify-between gap-3",
            label { class: "block min-w-72 flex-1 text-xs text-gray-600",
                span { class: "mb-1 block font-medium uppercase tracking-wide", "Active run" }
                select {
                    class: "w-full rounded-md border border-gray-300 px-3 py-2 text-sm",
                    value: "{current}",
                    onchange: move |event| on_select_run.call(event.value()),
                    for run in evidence.runs.iter() {
                        option {
                            value: "{run.run_id}",
                            selected: run.run_id == current || (current.is_empty() && run.run_id == evidence.run_id),
                            "{run.framework} · {run.run_id}"
                        }
                    }
                }
            }
            div { class: "flex items-center gap-3 text-xs text-gray-500",
                if let Some(step) = step_filter {
                    button {
                        class: "rounded border border-blue-200 bg-blue-50 px-2 py-1 font-medium text-blue-700 hover:bg-blue-100",
                        onclick: move |_| *SAMPLE_STEP_FILTER.write() = None,
                        "Clear step filter · {step}"
                    }
                }
                span { "{visible_samples.len()} samples" }
            }
        }
        if visible_samples.is_empty() {
            UnavailablePanel {
                label: "No samples match the current step filter".to_string(),
                detail: "Clear the step filter or wait for more rollout outcomes.".to_string(),
            }
        } else {
        div { class: "overflow-x-auto rounded-lg border border-gray-200",
            table { class: "w-full border-collapse text-xs",
                thead {
                    tr { class: "border-b border-gray-200 bg-gray-50 text-left uppercase tracking-wide text-gray-500",
                        th { class: "px-3 py-2 font-medium", "Step" }
                        th { class: "px-3 py-2 font-medium", "Task / group" }
                        th { class: "px-3 py-2 font-medium", "Rollout" }
                        th { class: "px-3 py-2 font-medium", "Status" }
                        th { class: "px-3 py-2 font-medium", "Reward" }
                        th { class: "px-3 py-2 font-medium", "Tokens" }
                        th { class: "px-3 py-2 font-medium", "Staleness" }
                        th { class: "px-3 py-2 font-medium", "Filter / drop" }
                        th { class: "px-3 py-2 font-medium", "Trace" }
                    }
                }
                tbody { class: "divide-y divide-gray-100",
                    for sample in visible_samples {
                        {
                            let rollout_id = sample.rollout_id.clone();
                            let reason = sample_reason(&sample);
                            rsx! {
                                tr { class: "hover:bg-gray-50/70",
                                    td { class: "px-3 py-2 text-gray-700", "{sample.step}" }
                                    td { class: "px-3 py-2",
                                        div { class: "font-medium text-gray-900", "{sample.task}" }
                                        div { class: "text-gray-500", "{sample.group_id}" }
                                    }
                                    td { class: "max-w-56 break-all px-3 py-2 font-mono text-gray-600", "{sample.rollout_id}" }
                                    td { class: "px-3 py-2 text-gray-700", "{sample.status}" }
                                    td { class: "px-3 py-2 text-gray-700", "{format_reward(sample.reward)}" }
                                    td { class: "px-3 py-2 text-gray-700", "{sample.prompt_tokens} + {sample.response_tokens}" }
                                    td { class: "px-3 py-2 text-gray-700", "{sample.staleness}" }
                                    td { class: "max-w-64 break-words px-3 py-2 text-gray-600", "{reason}" }
                                    td { class: "px-3 py-2",
                                        button {
                                            class: "font-medium text-blue-600 hover:underline",
                                            onclick: move |_| {
                                                *ROLLOUT_FILTER.write() = rollout_id.clone();
                                                *ROLLOUT_FILTER_INPUT.write() = rollout_id.clone();
                                                navigator.push(NextRoute::Rollout {});
                                            },
                                            "Open spans →"
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        }
    }
}

fn format_reward(reward: Option<f64>) -> String {
    reward
        .map(|value| format!("{value:.4}"))
        .unwrap_or_else(|| "—".to_string())
}

fn sample_reason(sample: &RlSampleSummary) -> String {
    if !sample.filter_reason.is_empty() {
        format!("filter: {}", sample.filter_reason)
    } else if !sample.drop_reason.is_empty() {
        format!("drop: {}", sample.drop_reason)
    } else {
        "—".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_reason_prefers_filter_reason() {
        let sample = RlSampleSummary {
            timestamp_ns: 0,
            step: 1,
            rollout_id: "r".into(),
            group_id: "g".into(),
            sample_id: "s".into(),
            task: "math".into(),
            status: "filtered".into(),
            reward: None,
            reward_pass: false,
            filter_reason: "duplicate".into(),
            drop_reason: "late".into(),
            prompt_tokens: 1,
            response_tokens: 2,
            staleness: 0,
        };
        assert_eq!(sample_reason(&sample), "filter: duplicate");
    }
}
