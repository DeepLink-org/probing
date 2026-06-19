//! Right-side Investigation Agent panel (playbook runner + chat).

use std::collections::HashMap;

use dioxus::prelude::*;

use crate::agent::{
    list_playbook_ids, load_playbook, resolve_playbook_id, run_playbook, select_playbook,
    summarize_run,
};
use crate::components::agent::step_card::{step_outcome_to_card, AgentPlaybookRunCard, AgentStepCard};
use crate::components::colors::colors;
use crate::components::icon::Icon;
use crate::state::agent::{
    push_agent_message, AgentMessage, AgentMessageKind, AGENT_INPUT, AGENT_MESSAGES, AGENT_PANEL_OPEN,
    AGENT_RUNNING,
};
use crate::state::investigation::INVESTIGATION_CONTEXT;
use crate::state::llm_config::{LlmConfig, LLM_CONFIG, LLM_SETTINGS_OPEN};

const QUICK_PLAYBOOKS: &[(&str, &str)] = &[
    ("health_overview", "Health"),
    ("training_hang", "Hang"),
    ("slow_rank", "Slow rank"),
    ("memory_leak", "Memory"),
    ("module_bottleneck", "Bottleneck"),
];

#[component]
pub fn AgentPanel() -> Element {
    if !*AGENT_PANEL_OPEN.read() {
        return rsx! {};
    }

    let scroll_anchor = use_signal(|| 0u32);
    let messages = AGENT_MESSAGES.read().clone();
    let input_val = AGENT_INPUT.read().clone();
    let running = *AGENT_RUNNING.read();

    use_effect(move || {
        let _ = scroll_anchor();
    });

    let llm_on = LLM_CONFIG.read().is_configured();

    rsx! {
        aside {
            class: "w-[min(420px,100vw)] shrink-0 flex flex-col border-l border-gray-200 bg-white shadow-lg z-40",
            header {
                class: "flex items-center gap-2 px-4 py-3 border-b border-gray-200 bg-gradient-to-r from-slate-50 to-blue-50/40",
                Icon { icon: &icondata::AiRobotOutlined, class: "w-5 h-5 text-blue-600 shrink-0" }
                div { class: "flex-1 min-w-0",
                    div { class: "text-sm font-semibold text-gray-900", "Investigation Agent" }
                    if llm_on {
                        div { class: "text-xs text-emerald-600 truncate", "LLM enabled · {LLM_CONFIG.read().model}" }
                    } else {
                        div { class: "text-xs text-gray-500 truncate", "Keyword mode · configure LLM in ⚙" }
                    }
                }
                button {
                    class: "p-1.5 rounded-md text-gray-500 hover:bg-gray-100",
                    title: "LLM settings (API key stored in this browser)",
                    onclick: move |_| *LLM_SETTINGS_OPEN.write() = true,
                    Icon { icon: &icondata::AiSettingOutlined, class: "w-4 h-4" }
                }
                button {
                    class: "p-1.5 rounded-md text-gray-500 hover:bg-gray-100",
                    title: "Close agent panel",
                    onclick: move |_| *AGENT_PANEL_OPEN.write() = false,
                    Icon { icon: &icondata::AiCloseOutlined, class: "w-4 h-4" }
                }
            }

            div {
                class: "px-3 py-2 border-b border-gray-100 flex flex-wrap gap-1.5",
                for (id, label) in QUICK_PLAYBOOKS {
                    button {
                        class: "px-2 py-1 text-xs rounded-md border border-gray-200 bg-gray-50 text-gray-700 hover:bg-blue-50 hover:border-blue-200 hover:text-blue-800 disabled:opacity-50",
                        disabled: running,
                        onclick: {
                            let pid = (*id).to_string();
                            move |_| spawn_run_playbook(pid.clone(), HashMap::new(), None)
                        },
                        "{label}"
                    }
                }
            }

            div {
                class: "flex-1 overflow-y-auto px-3 py-3 space-y-3 min-h-0",
                id: "agent-scroll",
                if messages.is_empty() {
                    AgentWelcome {}
                }
                for (idx, msg) in messages.iter().enumerate() {
                    AgentMessageView { key: "{idx}", message: msg.clone() }
                }
                div { id: "agent-scroll-anchor-{scroll_anchor()}" }
            }

            div {
                class: "p-3 border-t border-gray-200 bg-gray-50/80",
                div {
                    class: "flex gap-2",
                    input {
                        class: "flex-1 min-w-0 px-3 py-2 text-sm border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500 font-sans",
                        placeholder: "Describe issue or /health_overview …",
                        disabled: running,
                        value: "{input_val}",
                        oninput: move |e| *AGENT_INPUT.write() = e.value(),
                        onkeydown: move |e: dioxus::html::events::KeyboardEvent| {
                            use dioxus::html::input_data::keyboard_types::Key;
                            if e.key() == Key::Enter && !running {
                                submit_agent_input();
                            }
                        },
                    }
                    button {
                        class: format!(
                            "px-3 py-2 text-sm font-medium text-white rounded-lg bg-{} hover:opacity-90 disabled:opacity-50 shrink-0",
                            colors::PRIMARY
                        ),
                        disabled: running || AGENT_INPUT.read().trim().is_empty(),
                        onclick: move |_| submit_agent_input(),
                        if running { "…" } else { "Run" }
                    }
                }
                div { class: "mt-2 flex justify-between items-center text-[10px] text-gray-400",
                    span { "⌘J toggle · Enter run" }
                    button {
                        class: "text-gray-500 hover:text-gray-700 underline",
                        disabled: running,
                        onclick: move |_| {
                            crate::state::agent::clear_agent_messages();
                        },
                        "Clear"
                    }
                }
            }
        }
    }
}

#[component]
fn AgentWelcome() -> Element {
    let ctx = INVESTIGATION_CONTEXT.read().clone();
    rsx! {
        div {
            class: "rounded-lg border border-dashed border-gray-200 bg-gray-50/80 p-4 text-sm text-gray-600 space-y-2",
            p { class: "font-medium text-gray-800", "Ask in plain language or pick a quick playbook above." }
            ul { class: "list-disc list-inside text-xs space-y-1 text-gray-500",
                li { "「训练卡住了」→ training_hang" }
                li { "「哪个 rank 慢」→ slow_rank" }
                li { "「显存在涨」→ memory_leak" }
            }
            if !ctx.is_empty() {
                p { class: "text-xs text-blue-700 bg-blue-50 rounded px-2 py-1 font-mono",
                    "Context: {ctx.summary()}"
                }
            }
            if LLM_CONFIG.read().is_configured() {
                p { class: "text-xs text-emerald-700", "LLM will pick playbooks and summarize results." }
            } else {
                p { class: "text-xs text-gray-500",
                    "No LLM key — open ⚙ to save an API key in this browser (localStorage)."
                }
            }
            p { class: "text-xs text-gray-400",
                "Available: {list_playbook_ids().join(\", \")}"
            }
        }
    }
}

#[component]
fn AgentMessageView(message: AgentMessage) -> Element {
    match message.kind {
        AgentMessageKind::User => rsx! {
            div { class: "flex justify-end",
                div {
                    class: "max-w-[90%] px-3 py-2 rounded-lg bg-blue-600 text-white text-sm",
                    "{message.text}"
                }
            }
        },
        AgentMessageKind::Assistant => rsx! {
            AgentAssistantBlock { text: message.text.clone() }
        },
        AgentMessageKind::PlaybookRun => rsx! {
            AgentPlaybookRunCard {
                title: message.title.clone().unwrap_or_default(),
                playbook_id: message.playbook_id.clone().unwrap_or_default(),
                category: message.playbook_category.clone().unwrap_or_default(),
                docs: message.text.clone(),
            }
        },
        AgentMessageKind::StepCard => {
            if let Some(step) = message.step.clone() {
                rsx! { AgentStepCard { step } }
            } else {
                rsx! {}
            }
        },
        AgentMessageKind::Error => rsx! {
            div {
                class: "px-3 py-2 rounded-lg bg-red-50 border border-red-100 text-sm text-red-800",
                "{message.text}"
            }
        },
    }
}

#[component]
fn AgentAssistantBlock(text: String) -> Element {
    let chips = extract_playbook_chips(&text);
    rsx! {
        div { class: "space-y-2",
            div {
                class: "px-3 py-2 rounded-lg bg-gray-100 text-sm text-gray-800 whitespace-pre-wrap",
                "{text}"
            }
            if !chips.is_empty() {
                div { class: "flex flex-wrap gap-1.5",
                    for id in chips {
                        button {
                            class: "px-2 py-1 text-xs rounded-md border border-blue-200 bg-blue-50 text-blue-800 hover:bg-blue-100 disabled:opacity-50",
                            disabled: *AGENT_RUNNING.read(),
                            onclick: move |_| spawn_run_playbook(id.clone(), HashMap::new(), None),
                            "Run {id}"
                        }
                    }
                }
            }
        }
    }
}

fn extract_playbook_chips(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        if let Some(idx) = line.find("playbook:") {
            let rest = line[idx + "playbook:".len()..].trim();
            let id = rest.split_whitespace().next().unwrap_or("").trim();
            if load_playbook(id).is_some() && !out.contains(&id.to_string()) {
                out.push(id.to_string());
            }
        }
    }
    out
}

fn submit_agent_input() {
    let text = AGENT_INPUT.read().trim().to_string();
    if text.is_empty() || *AGENT_RUNNING.read() {
        return;
    }
    *AGENT_INPUT.write() = String::new();
    push_agent_message(AgentMessage::user(text.clone()));

    if text.starts_with('/') || text.starts_with("run ") || load_playbook(text.as_str()).is_some() {
        if let Some(id) = resolve_playbook_id(&text) {
            spawn_run_playbook(id, HashMap::new(), None);
            return;
        }
    }

    let llm_cfg = LLM_CONFIG.read().clone();
    if llm_cfg.is_configured() {
        spawn_llm_flow(text, llm_cfg);
        return;
    }

    if let Some(id) = resolve_playbook_id(&text) {
        spawn_run_playbook(id, HashMap::new(), None);
    } else {
        push_agent_message(AgentMessage::assistant(
            "No playbook matched. Try quick chips, /health_overview, or open ⚙ to add an LLM API key."
                .to_string(),
        ));
    }
}

fn spawn_llm_flow(user_message: String, config: LlmConfig) {
    if *AGENT_RUNNING.read() {
        return;
    }
    *AGENT_RUNNING.write() = true;
    spawn(async move {
        match select_playbook(&config, &user_message).await {
            Ok(sel) => {
                if !sel.reply.is_empty() {
                    push_agent_message(AgentMessage::assistant(sel.reply.clone()));
                }
                match sel.playbook_id {
                    Some(id) if load_playbook(&id).is_some() => {
                        run_playbook_flow(&id, sel.parameters, Some((config, user_message))).await;
                    }
                    Some(id) => {
                        push_agent_message(AgentMessage::error(format!(
                            "LLM chose unknown playbook: {id}"
                        )));
                    }
                    None => {
                        if sel.reply.is_empty() {
                            push_agent_message(AgentMessage::assistant(
                                "No suitable playbook — try rephrasing or pick a quick chip.".to_string(),
                            ));
                        }
                    }
                }
            }
            Err(e) => {
                push_agent_message(AgentMessage::error(format!(
                    "LLM error: {}\n\nCheck ⚙ settings (API base, key, CORS). Falling back: try /health_overview",
                    e.display_message()
                )));
            }
        }
        *AGENT_RUNNING.write() = false;
    });
}

fn spawn_run_playbook(
    playbook_id: String,
    overrides: HashMap<String, String>,
    llm_followup: Option<(LlmConfig, String)>,
) {
    if *AGENT_RUNNING.read() {
        return;
    }
    *AGENT_RUNNING.write() = true;
    spawn(async move {
        run_playbook_flow(&playbook_id, overrides, llm_followup).await;
        *AGENT_RUNNING.write() = false;
    });
}

async fn run_playbook_flow(
    playbook_id: &str,
    overrides: HashMap<String, String>,
    llm_followup: Option<(LlmConfig, String)>,
) {
    let Some(meta) = load_playbook(playbook_id) else {
        push_agent_message(AgentMessage::error(format!("Unknown playbook: {playbook_id}")));
        return;
    };

    push_agent_message(AgentMessage::playbook_run(
        meta.id.clone(),
        meta.title.clone(),
        meta.category.clone(),
        meta.docs.clone(),
    ));

    let overrides = if overrides.is_empty() {
        HashMap::new()
    } else {
        overrides
    };
    match run_playbook(playbook_id, overrides).await {
        Ok((pb, outcomes)) => {
            let evidence = crate::agent::outcomes_to_evidence(&outcomes);
            for outcome in outcomes {
                push_agent_message(AgentMessage::step_card(step_outcome_to_card(outcome)));
            }

            if let Some((config, user_msg)) = llm_followup {
                match summarize_run(&config, &user_msg, playbook_id, &evidence).await {
                    Ok(summary) => push_agent_message(AgentMessage::assistant(summary)),
                    Err(e) => push_agent_message(AgentMessage::error(format!(
                        "Summary failed: {}",
                        e.display_message()
                    ))),
                }
            } else {
                if !pb.summary_template.is_empty() {
                    push_agent_message(AgentMessage::assistant(pb.summary_template.clone()));
                }
                if !pb.next_steps.is_empty() {
                    let tips = pb
                        .next_steps
                        .iter()
                        .map(|s| format!("• {s}"))
                        .collect::<Vec<_>>()
                        .join("\n");
                    push_agent_message(AgentMessage::assistant(format!("**Next steps**\n{tips}")));
                }
            }
        }
        Err(e) => {
            push_agent_message(AgentMessage::error(e.display_message()));
        }
    }
}
