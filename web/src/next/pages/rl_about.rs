use dioxus::prelude::*;
use probing_proto::prelude::RlAboutResponse;

use crate::api::ApiClient;
use crate::hooks::{use_page_visible, use_polled_resource};
use crate::utils::error::Result;

use super::super::components::{
    EvidenceMetric, LoadingPanel, SectionCard, UnavailablePanel, WorkspacePage,
};
use super::super::rl_run::{read_selected_run_id, resolve_run, write_selected_run_id};

const POLL_MS: u32 = 10_000;

#[derive(Clone, Debug, PartialEq)]
struct AboutEvidence {
    runs: Vec<probing_proto::prelude::RlRunSummary>,
    about: RlAboutResponse,
}

#[component]
pub fn RlAboutPage() -> Element {
    let visible = use_page_visible();
    let mut selected_run_id = use_signal(|| read_selected_run_id().unwrap_or_default());
    let evidence = use_polled_resource(POLL_MS, Some(visible), move || {
        let preferred = selected_run_id();
        async move { load_about(preferred).await }
    });
    let state = evidence.read().clone();

    rsx! {
        WorkspacePage {
            title: "RL About".to_string(),
            subtitle: "Run identity, configuration hash, and adapter-reported experiment metadata.".to_string(),
            actions: rsx! { span { class: "text-xs text-gray-500", "Live · {POLL_MS / 1000}s" } },
            match state {
                None => rsx! { LoadingPanel { label: "Loading RL about metadata".to_string() } },
                Some(Err(error)) => rsx! { UnavailablePanel {
                    label: "RL about unavailable".to_string(),
                    detail: error.display_message(),
                }},
                Some(Ok(evidence)) => rsx! {
                    AboutContent {
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

async fn load_about(preferred: String) -> Result<AboutEvidence> {
    let client = ApiClient::new();
    let runs = client.fetch_rl_runs().await?.runs;
    let Some(run) = resolve_run(&runs, Some(preferred.as_str())) else {
        return Ok(AboutEvidence {
            runs,
            about: RlAboutResponse::default(),
        });
    };
    let about = client.fetch_rl_about(&run.run_id).await?;
    Ok(AboutEvidence { runs, about })
}

#[component]
fn AboutContent(
    evidence: AboutEvidence,
    selected_run_id: Signal<String>,
    on_select_run: EventHandler<String>,
) -> Element {
    let Some(run) = evidence.about.run.clone() else {
        return rsx! { UnavailablePanel {
            label: "No RL runs reported".to_string(),
            detail: "Enable probing.ext.rl_data and wait for the first trainer step.".to_string(),
        }};
    };
    let current = selected_run_id();
    let progress = progress_label(&evidence.about, run.global_step);

    rsx! {
        SectionCard {
            title: format!("{} · {}", run.framework, run.run_id),
            subtitle: Some("Experiment identity and progress.".to_string()),
            div { class: "border-b border-gray-200 px-4 py-3",
                label { class: "mb-1 block text-[11px] font-medium uppercase tracking-wide text-gray-500",
                    "Active run"
                }
                select {
                    class: "w-full max-w-xl rounded-md border border-gray-300 bg-white px-3 py-2 text-sm",
                    value: "{current}",
                    onchange: move |event| on_select_run.call(event.value()),
                    for candidate in evidence.runs.iter() {
                        option {
                            value: "{candidate.run_id}",
                            selected: candidate.run_id == current || (current.is_empty() && candidate.run_id == run.run_id),
                            "{candidate.framework} · {candidate.run_id}"
                        }
                    }
                }
            }
            div { class: "grid grid-cols-2 divide-x divide-gray-200 lg:grid-cols-4",
                EvidenceMetric { label: "Phase", value: nonempty(&run.phase) }
                EvidenceMetric { label: "Global step", value: run.global_step.to_string() }
                EvidenceMetric { label: "Progress", value: progress }
                EvidenceMetric { label: "Job ID", value: nonempty(&run.job_id) }
            }
        }

        SectionCard {
            title: "Configuration".to_string(),
            subtitle: Some("Stable identity fields reported by the framework adapter.".to_string()),
            div { class: "grid grid-cols-1 gap-2 p-4 sm:grid-cols-2",
                MetaRow { label: "Config hash", value: nonempty(&run.config_hash) }
                MetaRow { label: "Samples / tokens", value: format!("{} / {}", run.samples_total, run.tokens_total) }
                MetaRow { label: "Start ns", value: run.start_time_ns.to_string() }
                MetaRow { label: "End ns", value: if run.end_time_ns > 0 { run.end_time_ns.to_string() } else { "running".into() } }
            }
        }

        if !evidence.about.about_metrics.is_empty() {
            SectionCard {
                title: "About metrics".to_string(),
                subtitle: Some("Latest about.* scalars from the trainer.".to_string()),
                div { class: "grid grid-cols-2 gap-2 p-4 lg:grid-cols-4",
                    for (name, value) in evidence.about.about_metrics.iter() {
                        div { class: "rounded-lg border border-gray-200 bg-gray-50 px-3 py-2",
                            EvidenceMetric {
                                label: name.clone(),
                                value: format_metric(*value),
                            }
                        }
                    }
                }
            }
        }

        if !evidence.about.metadata.is_empty() {
            SectionCard {
                title: "Hyperparameters / metadata".to_string(),
                subtitle: Some("JSON metadata attached to the run snapshot.".to_string()),
                div { class: "overflow-x-auto",
                    table { class: "min-w-full divide-y divide-gray-200 text-left text-xs",
                        thead { class: "bg-gray-50 text-[11px] uppercase tracking-wide text-gray-500",
                            tr {
                                th { class: "px-3 py-2 font-medium", "Key" }
                                th { class: "px-3 py-2 font-medium", "Value" }
                            }
                        }
                        tbody { class: "divide-y divide-gray-100 bg-white",
                            for (key, value) in evidence.about.metadata.iter() {
                                tr {
                                    td { class: "px-3 py-2 font-medium text-gray-800", "{key}" }
                                    td { class: "px-3 py-2 font-mono text-gray-700", "{value}" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn MetaRow(label: String, value: String) -> Element {
    rsx! {
        div { class: "rounded-md border border-gray-200 px-3 py-2 text-xs",
            div { class: "text-[11px] uppercase tracking-wide text-gray-500", "{label}" }
            div { class: "mt-1 break-all font-medium text-gray-800", "{value}" }
        }
    }
}

fn progress_label(about: &RlAboutResponse, global_step: i64) -> String {
    if let Some(progress) = about.about_metrics.get("about.progress") {
        return format!("{:.1}%", progress * 100.0);
    }
    if let Some(target) = about.about_metrics.get("about.target_steps") {
        if *target > 0.0 {
            return format!("{:.1}%", 100.0 * global_step as f64 / target);
        }
    }
    "—".to_string()
}

fn format_metric(value: f64) -> String {
    if value.abs() >= 1_000.0 {
        format!("{value:.0}")
    } else if value.abs() >= 10.0 {
        format!("{value:.2}")
    } else {
        format!("{value:.4}")
    }
}

fn nonempty(value: &str) -> String {
    if value.is_empty() {
        "—".to_string()
    } else {
        value.to_string()
    }
}
