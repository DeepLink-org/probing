use dioxus::prelude::*;
use dioxus_router::use_navigator;

use crate::components::agent::chat::{submit_agent_text, trigger_skill};
use crate::components::dataframe_view::DataFrameView;
use crate::components::markdown_view::MarkdownView;
use crate::components::source_viewer::SourceRefChip;
use crate::state::agent::{
    clear_agent_messages, AgentMessage, AgentMessageKind, AgentStepStatus, AGENT_INPUT,
    AGENT_MESSAGES,
};
use crate::state::investigation::INVESTIGATION_CONTEXT;
use crate::state::llm_config::LLM_SETTINGS_OPEN;
use crate::state::ui_tasks::{ui_agent_busy, UI_TASK_TICK};

use super::super::components::{ActionButton, SectionCard, WorkspacePage};
use super::super::routes::NextRoute;

const QUICK_SKILLS: &[(&str, &str)] = &[
    ("health_overview", "Health overview"),
    ("training_hang", "Training hang"),
    ("slow_rank", "Slow rank"),
    ("nccl_culprit_victim", "NCCL culprit"),
    ("comm_bottleneck", "Communication"),
    ("memory_leak", "Memory leak"),
    ("module_bottleneck", "Module bottleneck"),
];

#[component]
pub fn InvestigatePage() -> Element {
    let messages = AGENT_MESSAGES.read().clone();
    let busy = ui_agent_busy();
    let context = INVESTIGATION_CONTEXT.read().clone();

    rsx! {
        WorkspacePage {
            title: "Investigate".to_string(),
            subtitle: "Collect evidence with deterministic skills, preserve context, and keep rule output separate from summaries.".to_string(),
            actions: rsx! {
                    ActionButton {
                        label: "LLM settings".to_string(),
                        onclick: move |_| *LLM_SETTINGS_OPEN.write() = true,
                    }
                    ActionButton {
                        label: "Clear session".to_string(),
                        disabled: busy || messages.is_empty(),
                        onclick: move |_| clear_agent_messages(),
                    }
                },

            div { class: "grid gap-4 lg:grid-cols-2",
                SectionCard {
                    title: "Pinned context".to_string(),
                    body_class: "p-3".to_string(),
                    if context.is_empty() {
                        p { class: "text-xs text-gray-500", "No rank, step, trace, or process has been pinned yet." }
                    } else {
                        div { class: "space-y-2 text-sm",
                            div { class: "rounded-lg bg-blue-50 px-3 py-2 font-mono text-xs text-blue-900", "{context.summary()}" }
                            if let Some(label) = context.label {
                                p { class: "text-xs text-gray-600", "{label}" }
                            }
                        }
                    }
                }
                SectionCard {
                    title: "Evidence rules".to_string(),
                    body_class: "p-3".to_string(),
                    div { class: "grid gap-2 text-xs leading-relaxed text-gray-600 sm:grid-cols-3 lg:grid-cols-1 xl:grid-cols-3",
                        p { "Failures stay failures; they are never rendered as healthy empty results." }
                        p { "Partial cluster results retain coverage and missing-peer metadata." }
                        p { "Skill rules produce findings; the LLM only summarizes and selects next steps." }
                    }
                }
            }

            InvestigateSession { compact: false }
        }
    }
}

#[component]
pub(crate) fn InvestigateSession(compact: bool) -> Element {
    let messages = AGENT_MESSAGES.read().clone();
    let input = AGENT_INPUT.read().clone();
    let _task_tick = UI_TASK_TICK.read();
    let busy = ui_agent_busy();
    let body_class = if compact {
        "flex min-h-0 flex-1 flex-col p-0"
    } else {
        "flex min-h-[36rem] flex-col p-0"
    };
    let submit = move || {
        let text = AGENT_INPUT.read().trim().to_string();
        if text.is_empty() || ui_agent_busy() {
            return;
        }
        *AGENT_INPUT.write() = String::new();
        submit_agent_text(text);
    };

    rsx! {
        SectionCard {
            title: "Diagnostic session".to_string(),
            subtitle: Some(if busy { "A diagnostic skill is running.".to_string() } else { "Ask in plain language or run a focused skill.".to_string() }),
            body_class: body_class.to_string(),
            fill: compact,
            div { class: "border-b border-gray-100 p-3",
                div { class: "flex flex-wrap gap-2",
                    for (id, label) in QUICK_SKILLS {
                        button {
                            r#type: "button",
                            class: "rounded-full border border-gray-300 bg-white px-3 py-1.5 text-xs font-medium text-gray-700 hover:border-blue-300 hover:bg-blue-50 disabled:opacity-50",
                            disabled: busy,
                            onclick: {
                                let id = (*id).to_string();
                                move |_| trigger_skill(id.clone())
                            },
                            "{label}"
                        }
                    }
                }
            }
            div { class: "min-h-0 flex-1 space-y-3 overflow-y-auto bg-gray-50/60 p-4",
                if messages.is_empty() {
                    div { class: "mx-auto max-w-2xl rounded-xl border border-dashed border-gray-300 bg-white px-5 py-8 text-center",
                        h2 { class: "text-base font-semibold text-gray-900", "Start from the observed symptom" }
                        p { class: "mt-2 text-sm leading-relaxed text-gray-500",
                            "Examples: “Which rank is slow?”, “Why did step latency increase?”, or “Is NCCL the culprit or a victim?”"
                        }
                    }
                }
                for (index, message) in messages.iter().enumerate() {
                    NextAgentMessage { key: "{index}", message: message.clone() }
                }
                if busy {
                    div { class: "flex items-center gap-2 px-2 py-3 text-xs text-gray-500",
                        span { class: "h-3.5 w-3.5 animate-spin rounded-full border-2 border-blue-500 border-t-transparent motion-reduce:animate-none", aria_hidden: "true" }
                        "Collecting and interpreting evidence…"
                    }
                }
            }
            div { class: "border-t border-gray-100 bg-white p-3",
                div { class: "flex gap-2",
                    input {
                        class: "min-w-0 flex-1 rounded-lg border border-gray-300 px-3 py-2 text-sm focus:border-blue-500 focus:outline-none focus:ring-2 focus:ring-blue-100",
                        placeholder: "Describe the issue or /health_overview",
                        value: "{input}",
                        disabled: busy,
                        oninput: move |event| *AGENT_INPUT.write() = event.value(),
                        onkeydown: move |event: dioxus::html::events::KeyboardEvent| {
                            use dioxus::html::input_data::keyboard_types::Key;
                            if event.key() == Key::Enter {
                                submit();
                            }
                        },
                    }
                    button {
                        r#type: "button",
                        class: "rounded-lg bg-blue-600 px-4 py-2 text-sm font-medium text-white hover:bg-blue-700 disabled:opacity-50",
                        disabled: busy || AGENT_INPUT.read().trim().is_empty(),
                        onclick: move |_| submit(),
                        if busy { "Running…" } else { "Run" }
                    }
                }
            }
        }
    }
}

#[component]
fn NextAgentMessage(message: AgentMessage) -> Element {
    let navigator = use_navigator();
    match message.kind {
        AgentMessageKind::User => rsx! {
            div { class: "flex justify-end",
                div { class: "max-w-[85%] rounded-xl bg-blue-600 px-3 py-2 text-sm text-white", "{message.text}" }
            }
        },
        AgentMessageKind::Assistant => {
            let references = crate::utils::source_ref::extract_source_refs(&message.text);
            let skill_ids = extract_skill_ids(&message.text);
            rsx! {
                div { class: "space-y-2",
                    div { class: "max-w-[92%] rounded-xl border border-gray-200 bg-white px-3 py-2 shadow-sm",
                        MarkdownView { content: message.text }
                    }
                    if !references.is_empty() {
                        div { class: "flex flex-wrap gap-1.5",
                            for (index, reference) in references.iter().enumerate() {
                                SourceRefChip {
                                    key: "{index}",
                                    path: reference.path.clone(),
                                    line: reference.line.map(i64::from),
                                }
                            }
                        }
                    }
                    if !skill_ids.is_empty() {
                        div { class: "flex flex-wrap gap-1.5",
                            for id in skill_ids {
                                button {
                                    r#type: "button",
                                    class: "rounded-full border border-blue-200 bg-blue-50 px-2.5 py-1 text-xs font-medium text-blue-700 hover:bg-blue-100",
                                    onclick: move |_| trigger_skill(id.clone()),
                                    "Run {id}"
                                }
                            }
                        }
                    }
                }
            }
        }
        AgentMessageKind::SkillRun => {
            let category = message
                .skill_category
                .clone()
                .unwrap_or_else(|| "skill".to_string());
            let title = message
                .title
                .clone()
                .unwrap_or_else(|| message.skill_id.clone().unwrap_or_default());
            rsx! {
                div { class: "rounded-xl border border-blue-200 bg-blue-50 px-3 py-3",
                    div { class: "text-xs font-medium uppercase tracking-wide text-blue-600", "{category}" }
                    div { class: "mt-1 text-sm font-semibold text-blue-950", "{title}" }
                    if !message.text.is_empty() {
                        div { class: "mt-2 text-xs text-blue-900/75", MarkdownView { content: message.text } }
                    }
                }
            }
        }
        AgentMessageKind::StepCard => {
            let Some(step) = message.step else {
                return rsx! {};
            };
            let tone = match step.status {
                AgentStepStatus::Ok => "border-emerald-200",
                AgentStepStatus::Warn => "border-amber-200",
                AgentStepStatus::Skipped => "border-gray-200",
                AgentStepStatus::Error => "border-red-200",
            };
            rsx! {
                div { class: "rounded-xl border bg-white p-3 shadow-sm {tone}",
                    div { class: "flex flex-wrap items-center justify-between gap-2",
                        div { class: "text-sm font-semibold text-gray-900", "{step.title}" }
                        if let Some(rows) = step.row_count {
                            span { class: "rounded-full bg-gray-100 px-2 py-0.5 text-xs text-gray-600", "{rows} rows" }
                        }
                    }
                    if let Some(note) = step.cluster_note {
                        p { class: "mt-1 text-xs text-amber-700", "{note}" }
                    }
                    if !step.body_text.is_empty() {
                        p { class: "mt-2 whitespace-pre-wrap text-xs text-gray-600", "{step.body_text}" }
                    }
                    if let Some(dataframe) = step.dataframe {
                        div { class: "mt-3 max-h-64 overflow-auto rounded-lg border border-gray-200",
                            DataFrameView { df: dataframe }
                        }
                    }
                    if let Some(view) = step.navigate_view {
                        if let Some(route) = next_route_for_view(&view) {
                            button {
                                r#type: "button",
                                class: "mt-3 rounded-lg border border-blue-200 bg-blue-50 px-3 py-1.5 text-xs font-medium text-blue-700 hover:bg-blue-100",
                                onclick: move |_| {
                                    navigator.push(route.clone());
                                    *crate::state::agent::AGENT_PANEL_OPEN.write() = false;
                                },
                                "Open {view} evidence"
                            }
                        }
                    }
                }
            }
        }
        AgentMessageKind::Error => rsx! {
            div { class: "rounded-xl border border-red-200 bg-red-50 px-3 py-2 text-sm text-red-800 whitespace-pre-wrap",
                "{message.text}"
            }
        },
    }
}

fn extract_skill_ids(text: &str) -> Vec<String> {
    let mut ids = Vec::new();
    for line in text.lines() {
        let Some(index) = line.find("skill:") else {
            continue;
        };
        let id = line[index + "skill:".len()..]
            .split_whitespace()
            .next()
            .unwrap_or("")
            .trim_matches(|character: char| !character.is_ascii_alphanumeric() && character != '_');
        if !id.is_empty()
            && crate::agent::load_skill(id).is_some()
            && !ids.iter().any(|existing| existing == id)
        {
            ids.push(id.to_string());
        }
    }
    ids
}

fn next_route_for_view(view: &str) -> Option<NextRoute> {
    let normalized = view.trim().trim_matches('/');
    match normalized {
        "analytics" => Some(NextRoute::Analytics {}),
        "pprof" => Some(NextRoute::ProfileView {
            view: "pprof".to_string(),
        }),
        "torch" => Some(NextRoute::ProfileView {
            view: "torch".to_string(),
        }),
        "trace" | "chrome-trace" => Some(NextRoute::ProfileView {
            view: "trace".to_string(),
        }),
        "traces" | "spans" => Some(NextRoute::Spans {}),
        "python" => Some(NextRoute::Python {}),
        "training" => Some(NextRoute::Training {}),
        "cluster" => Some(NextRoute::Cluster {}),
        "stack" | "stacks" => Some(NextRoute::Stack {}),
        "rollout" | "rl" => Some(NextRoute::Rollout {}),
        "inference" => Some(NextRoute::Inference {}),
        other if other.starts_with("profiling/") => Some(NextRoute::ProfileView {
            view: other.trim_start_matches("profiling/").to_string(),
        }),
        _ => None,
    }
}
