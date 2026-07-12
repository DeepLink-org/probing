//! Shared Investigation Agent chat (floating overlay + full page).

use std::collections::HashMap;

use dioxus::prelude::*;

use crate::agent::{
    evaluate_rules_for_skill, format_findings, list_skill_ids, load_skill,
    refresh_page_snapshot_for_route, resolve_skill_id, run_skill, select_skill, skill_store_loaded,
    summarize_run,
};
use crate::api::ApiClient;
use crate::app::Route;
use crate::components::agent::step_card::{step_outcome_to_card, AgentSkillRunCard, AgentStepCard};
use crate::components::colors::colors;
use crate::components::icon::Icon;
use crate::components::markdown_view::MarkdownView;
use crate::components::source_viewer::SourceRefChip;
use crate::components::workspace::{
    ChipButton, SurfaceCard, SurfaceCardBody, WidthSegment, WorkspacePanelShell,
};
use crate::state::agent::{
    push_agent_message, save_agent_panel_width, take_agent_pending_action, AgentMessage,
    AgentMessageKind, AgentPanelWidth, AgentPendingAction, AGENT_ACTION_TICK, AGENT_INPUT,
    AGENT_MESSAGES, AGENT_PANEL_OPEN, AGENT_PANEL_WIDTH,
};
use crate::state::investigation::INVESTIGATION_CONTEXT;
use crate::state::llm_config::{LlmConfig, LLM_CONFIG, LLM_SETTINGS_OPEN};
use crate::state::page_context::PAGE_CONTEXT;
use crate::state::ui_tasks::{ui_agent_busy, UiTaskKind, UiTaskSession, UI_TASK_TICK};
use dioxus_router::use_route;

const QUICK_SKILLS: &[(&str, &str)] = &[
    ("health_overview", "Health"),
    ("training_hang", "Hang"),
    ("slow_rank", "Slow rank"),
    ("nccl_culprit_victim", "NCCL"),
    ("comm_bottleneck", "Comm"),
    ("memory_leak", "Memory"),
    ("module_bottleneck", "Bottleneck"),
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AgentChatVariant {
    /// Fixed overlay on the right; does not resize main content.
    Floating,
    /// Full workspace page at `/agent`.
    Page,
}

#[component]
pub fn AgentChat(variant: AgentChatVariant) -> Element {
    let scroll_anchor = use_signal(|| 0u32);
    let messages = AGENT_MESSAGES.read().clone();
    let input_val = AGENT_INPUT.read().clone();
    let _task_tick = UI_TASK_TICK.read();
    let _agent_action_tick = AGENT_ACTION_TICK.read();
    let running = ui_agent_busy();
    let panel_width = *AGENT_PANEL_WIDTH.read();
    let llm_on = LLM_CONFIG.read().is_configured();

    use_effect(move || {
        let _ = scroll_anchor();
        let _ = _agent_action_tick;
        let Some(action) = take_agent_pending_action() else {
            return;
        };
        match action {
            AgentPendingAction::SubmitText(text) => submit_agent_text(text),
            AgentPendingAction::RunSkill(id) => trigger_skill(id),
        }
    });

    let subtitle = if llm_on {
        format!("LLM · {}", LLM_CONFIG.read().model)
    } else {
        "Keyword mode · ⚙ for LLM".to_string()
    };

    let settings_btn = rsx! {
        button {
            class: "p-1.5 rounded-md text-gray-500 hover:bg-gray-100",
            title: "LLM settings",
            onclick: move |_| *LLM_SETTINGS_OPEN.write() = true,
            Icon { icon: &icondata::AiSettingOutlined, class: "w-4 h-4" }
        }
    };

    let header_actions = match variant {
        AgentChatVariant::Floating => rsx! {
            div { class: "inline-flex items-center rounded-lg border border-gray-200 bg-gray-100 p-0.5",
                WidthSegment {
                    label: "⅓",
                    selected: panel_width == AgentPanelWidth::Third,
                    title: "Overlay width 1/3 of viewport",
                    onclick: move |_| save_agent_panel_width(AgentPanelWidth::Third),
                }
                WidthSegment {
                    label: "⅔",
                    selected: panel_width == AgentPanelWidth::TwoThirds,
                    title: "Overlay width 2/3 of viewport",
                    onclick: move |_| save_agent_panel_width(AgentPanelWidth::TwoThirds),
                }
            }
            {settings_btn}
            button {
                class: "p-1.5 rounded-md text-gray-500 hover:bg-gray-100",
                title: "Close overlay",
                onclick: move |_| *AGENT_PANEL_OPEN.write() = false,
                Icon { icon: &icondata::AiCloseOutlined, class: "w-4 h-4" }
            }
        },
        AgentChatVariant::Page => rsx! {
            {settings_btn}
        },
    };

    let toolbar = rsx! {
        div { class: "flex flex-wrap gap-1.5",
            for (id, label) in QUICK_SKILLS {
                ChipButton {
                    label: (*label).to_string(),
                    disabled: running,
                    onclick: {
                        let skill_id = (*id).to_string();
                        move |_| spawn_run_skill(skill_id.clone(), HashMap::new(), None)
                    },
                }
            }
        }
    };

    let footer = rsx! {
        div {
            class: "flex gap-2",
            input {
                class: "flex-1 min-w-0 px-3 py-2 text-sm border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500 font-sans bg-white",
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
            span {
                if variant == AgentChatVariant::Floating {
                    "⌘J toggle overlay · Enter run"
                } else {
                    "Full-page Investigate · Enter run"
                }
            }
            button {
                class: "text-gray-500 hover:text-gray-700 underline",
                disabled: running,
                onclick: move |_| crate::state::agent::clear_agent_messages(),
                "Clear"
            }
        }
    };

    rsx! {
        WorkspacePanelShell {
            title: "Investigate".to_string(),
            subtitle: Some(subtitle),
            icon: &icondata::AiRobotOutlined,
            header_actions: Some(header_actions),
            toolbar: Some(toolbar),
            footer: footer,
            embedded: variant == AgentChatVariant::Page,
            div { id: "agent-scroll",
                AgentPageContextCard {}
                if messages.is_empty() {
                    AgentWelcome {}
                }
                for (idx, msg) in messages.iter().enumerate() {
                    AgentMessageView { key: "{idx}", message: msg.clone() }
                }
                if running {
                    AgentWorkingIndicator {}
                }
                div { id: "agent-scroll-anchor-{scroll_anchor()}" }
            }
        }
    }
}

#[component]
fn AgentPageContextCard() -> Element {
    let page = PAGE_CONTEXT.read().clone();
    let route = use_route::<Route>();

    rsx! {
        SurfaceCard {
            SurfaceCardBody {
                class: "px-3 py-2 space-y-1.5 text-xs",
                div { class: "flex items-start justify-between gap-2",
                    div { class: "min-w-0",
                        div { class: "font-medium text-gray-900", "Viewing: {page.title}" }
                        div { class: "text-[10px] text-gray-500 font-mono truncate", "{page.path}" }
                        p { class: "text-gray-600 mt-1 leading-relaxed", "{page.description}" }
                    }
                    button {
                        class: "shrink-0 px-2 py-1 text-[10px] rounded border border-gray-200 text-gray-600 hover:bg-gray-50",
                        title: "Refresh page snapshot for LLM",
                        disabled: page.snapshot_loading,
                        onclick: move |_| {
                            let route = route.clone();
                            spawn(async move {
                                refresh_page_snapshot_for_route(route).await;
                            });
                        },
                        if page.snapshot_loading { "…" } else { "Refresh" }
                    }
                }
                if !page.suggested_skills.is_empty() {
                    div { class: "flex flex-wrap gap-1 pt-0.5",
                        span { class: "text-[10px] text-gray-400", "Suggested:" }
                        for id in page.suggested_skills.iter().cloned() {
                            ChipButton {
                                label: id.clone(),
                                disabled: ui_agent_busy(),
                                onclick: move |_| spawn_run_skill(id.clone(), HashMap::new(), None),
                            }
                        }
                    }
                }
                if !page.snapshot.is_empty() {
                    pre { class: "text-[10px] font-mono text-gray-700 bg-gray-50 rounded p-2 max-h-32 overflow-auto whitespace-pre-wrap border border-gray-100",
                        "{page.snapshot}"
                    }
                } else if page.snapshot_loading {
                    p { class: "text-[10px] text-gray-400", "Loading page snapshot…" }
                }
            }
        }
    }
}

#[component]
fn AgentWelcome() -> Element {
    let ctx = INVESTIGATION_CONTEXT.read().clone();
    rsx! {
        SurfaceCard {
            SurfaceCardBody {
                class: "px-3 py-3 space-y-2 text-sm text-gray-600",
                p { class: "font-medium text-gray-800", "Ask in plain language or pick a quick skill above." }
                ul { class: "list-disc list-inside text-xs space-y-1 text-gray-500",
                    li { "「训练卡住了」→ training_hang" }
                    li { "「哪个 rank 慢」→ slow_rank（多机自动 cluster fan-out）" }
                    li { "「通信慢 / NCCL」→ comm_bottleneck" }
                    li { "「显存在涨」→ memory_leak" }
                }
                if !ctx.is_empty() {
                    p { class: "text-xs text-blue-700 bg-blue-50 rounded px-2 py-1 font-mono",
                        "Context: {ctx.summary()}"
                    }
                }
                if LLM_CONFIG.read().is_configured() {
                    p { class: "text-xs text-emerald-700", "LLM will pick skills and summarize results." }
                } else {
                    p { class: "text-xs text-gray-500",
                        "No LLM key — open ⚙ to save an API key in this browser (localStorage)."
                    }
                }
                p { class: "text-xs text-gray-400",
                    "Available: {list_skill_ids().join(\", \")}"
                }
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
                    class: "max-w-[90%] px-3 py-2 rounded-lg bg-blue-600 text-white text-sm shadow-sm",
                    "{message.text}"
                }
            }
        },
        AgentMessageKind::Assistant => rsx! {
            AgentAssistantBlock { text: message.text.clone() }
        },
        AgentMessageKind::SkillRun => rsx! {
            AgentSkillRunCard {
                title: message.title.clone().unwrap_or_default(),
                skill_id: message.skill_id.clone().unwrap_or_default(),
                category: message.skill_category.clone().unwrap_or_default(),
                docs: message.text.clone(),
            }
        },
        AgentMessageKind::StepCard => {
            if let Some(step) = message.step.clone() {
                rsx! { AgentStepCard { step } }
            } else {
                rsx! { div {} }
            }
        }
        AgentMessageKind::Error => rsx! {
            SurfaceCard {
                SurfaceCardBody {
                    class: "px-3 py-2 text-sm text-red-800 bg-red-50 whitespace-pre-wrap",
                    "{message.text}"
                }
            }
        },
    }
}

#[component]
fn AgentAssistantBlock(text: String) -> Element {
    let _task_tick = UI_TASK_TICK.read();
    let agent_busy = ui_agent_busy();
    let chips = extract_skill_chips(&text);
    let source_refs = crate::utils::source_ref::extract_source_refs(&text);
    rsx! {
        div { class: "space-y-2",
            SurfaceCard {
                SurfaceCardBody {
                    class: "px-3 py-2 bg-gray-50/50",
                    MarkdownView { content: text }
                }
            }
            if !source_refs.is_empty() {
                div { class: "flex flex-wrap gap-1.5",
                    for (i, reference) in source_refs.iter().enumerate() {
                        SourceRefChip {
                            key: "{i}",
                            path: reference.path.clone(),
                            line: reference.line.map(i64::from),
                        }
                    }
                }
            }
            if !chips.is_empty() {
                div { class: "flex flex-wrap gap-1.5",
                    for id in chips {
                        ChipButton {
                            label: format!("Run {id}"),
                            disabled: agent_busy,
                            onclick: move |_| spawn_run_skill(id.clone(), HashMap::new(), None),
                        }
                    }
                }
            }
        }
    }
}

fn extract_skill_chips(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        if let Some(idx) = line.find("skill:") {
            let rest = line[idx + "skill:".len()..].trim();
            let id = rest.split_whitespace().next().unwrap_or("").trim();
            if load_skill(id).is_some() && !out.contains(&id.to_string()) {
                out.push(id.to_string());
            }
        }
    }
    out
}

#[component]
fn AgentWorkingIndicator() -> Element {
    rsx! {
        div { class: "flex items-center gap-2 px-3 py-2 text-xs text-gray-500",
            span { class: "inline-block w-3 h-3 border-2 border-blue-500 border-t-transparent rounded-full animate-spin" }
            "Agent working — selecting skill and running diagnostics…"
        }
    }
}

/// Submit a user message to the agent (opens flow / skills / LLM).
pub fn submit_agent_text(text: String) {
    if text.trim().is_empty() || ui_agent_busy() {
        return;
    }
    dispatch_agent_message(text);
}

/// Run a skill immediately (used from source bridge, chips, etc.).
pub fn trigger_skill(skill_id: String) {
    if ui_agent_busy() {
        return;
    }
    spawn_run_skill(skill_id, HashMap::new(), None);
}

fn submit_agent_input() {
    let text = AGENT_INPUT.read().trim().to_string();
    if text.is_empty() || ui_agent_busy() {
        return;
    }
    *AGENT_INPUT.write() = String::new();
    dispatch_agent_message(text);
}

fn dispatch_agent_message(text: String) {
    push_agent_message(AgentMessage::user(text.clone()));

    if text.starts_with('/') || text.starts_with("run ") || load_skill(text.as_str()).is_some() {
        if let Some(id) = resolve_skill_id(&text) {
            spawn_run_skill(id, HashMap::new(), None);
            return;
        }
    }

    let llm_cfg = LLM_CONFIG.read().clone();
    if llm_cfg.is_configured() {
        spawn_llm_flow(text, llm_cfg);
        return;
    }

    if let Some(id) = resolve_skill_id(&text) {
        spawn_run_skill(id, HashMap::new(), None);
    } else {
        push_agent_message(AgentMessage::assistant(
            "No skill matched. Try quick chips, /health_overview, or open ⚙ to add an LLM API key."
                .to_string(),
        ));
    }
}

fn spawn_llm_flow(user_message: String, config: LlmConfig) {
    if ui_agent_busy() {
        return;
    }
    spawn(async move {
        let session = UiTaskSession::start();

        let wait = session.open(UiTaskKind::Agent, "Waiting for page context", None);
        let mut waited = 0u32;
        while PAGE_CONTEXT.read().snapshot_loading && waited < 8_000 {
            if wait.is_cancelled() {
                wait.cancel();
                return;
            }
            gloo_timers::future::TimeoutFuture::new(100).await;
            waited += 100;
        }
        if wait.is_cancelled() {
            wait.cancel();
            return;
        }
        wait.finish();

        if session.is_cancelled() {
            return;
        }

        let llm_task = session.open(UiTaskKind::Agent, "Select skill", None);
        match select_skill(&config, &user_message).await {
            Ok(sel) => {
                if llm_task.is_cancelled() {
                    llm_task.cancel();
                    return;
                }
                llm_task.finish();
                if !sel.reply.is_empty() {
                    push_agent_message(AgentMessage::assistant(sel.reply.clone()));
                }
                match sel.skill_id {
                    Some(id) if load_skill(&id).is_some() => {
                        run_skill_flow(&session, &id, sel.parameters, Some((config, user_message)))
                            .await;
                    }
                    Some(id) => {
                        push_agent_message(AgentMessage::error(format!(
                            "LLM chose unknown skill: {id}"
                        )));
                    }
                    None => {
                        if sel.reply.is_empty() {
                            push_agent_message(AgentMessage::assistant(
                                "No suitable skill — try rephrasing or pick a quick chip."
                                    .to_string(),
                            ));
                        }
                    }
                }
            }
            Err(e) => {
                if llm_task.is_cancelled() {
                    llm_task.cancel();
                } else {
                    llm_task.fail(e.display_message());
                    push_agent_message(AgentMessage::error(format!(
                        "LLM error: {}\n\nCheck ⚙ settings (API base, key, CORS). Falling back: try /health_overview",
                        e.display_message()
                    )));
                }
            }
        }
    });
}

fn spawn_run_skill(
    skill_id: String,
    overrides: HashMap<String, String>,
    llm_followup: Option<(LlmConfig, String)>,
) {
    if ui_agent_busy() {
        return;
    }
    spawn(async move {
        let session = UiTaskSession::start();
        run_skill_flow(&session, &skill_id, overrides, llm_followup).await;
    });
}

async fn ensure_skill_store_ready() -> std::result::Result<(), String> {
    if skill_store_loaded() {
        return Ok(());
    }
    ApiClient::new()
        .load_skill_store()
        .await
        .map_err(|e| e.to_string())
}

async fn run_skill_flow(
    session: &UiTaskSession,
    skill_id: &str,
    overrides: HashMap<String, String>,
    llm_followup: Option<(LlmConfig, String)>,
) {
    if session.is_cancelled() {
        return;
    }

    if let Err(msg) = ensure_skill_store_ready().await {
        push_agent_message(AgentMessage::error(format!(
            "Skill catalog not loaded: {msg}. Check that the probing server is running."
        )));
        return;
    }

    let Some(meta) = load_skill(skill_id) else {
        push_agent_message(AgentMessage::error(format!("Unknown skill: {skill_id}")));
        return;
    };

    let cluster = crate::agent::fetch_cluster_snapshot().await;
    if session.is_cancelled() {
        return;
    }
    if cluster.is_distributed() {
        push_agent_message(AgentMessage::assistant(format!(
            "Cluster: {} node(s), {} peer(s) — global.* SQL will fan out across nodes.",
            cluster.node_count, cluster.peer_count
        )));
    }

    push_agent_message(AgentMessage::skill_run(
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
    match run_skill(skill_id, overrides, Some(session)).await {
        Ok((pb, outcomes, ctx)) => {
            if session.is_cancelled() {
                return;
            }
            let findings = evaluate_rules_for_skill(&pb, &outcomes, &ctx);
            let evidence = crate::agent::outcomes_to_evidence(&outcomes);
            let fallback_summary = if llm_followup.is_none() {
                crate::agent::build_skill_summary(&pb, &outcomes, &ctx)
            } else {
                String::new()
            };
            for outcome in outcomes {
                push_agent_message(AgentMessage::step_card(step_outcome_to_card(outcome)));
            }
            let findings_text = format_findings(&findings);
            if !findings_text.is_empty() {
                push_agent_message(AgentMessage::assistant(findings_text));
            }

            if let Some((config, user_msg)) = llm_followup {
                let summary_task = session.open(
                    UiTaskKind::Agent,
                    "Summarize results",
                    Some(skill_id.to_string()),
                );
                match summarize_run(&config, &user_msg, skill_id, &evidence).await {
                    Ok(summary) => {
                        if summary_task.is_cancelled() {
                            summary_task.cancel();
                            return;
                        }
                        summary_task.finish();
                        push_agent_message(AgentMessage::assistant(summary));
                    }
                    Err(e) => {
                        if summary_task.is_cancelled() {
                            summary_task.cancel();
                        } else {
                            summary_task.fail(e.display_message());
                            push_agent_message(AgentMessage::error(format!(
                                "Summary failed: {}",
                                e.display_message()
                            )));
                        }
                    }
                }
            } else {
                if !fallback_summary.is_empty() {
                    push_agent_message(AgentMessage::assistant(fallback_summary));
                } else if !pb.summary_template.is_empty() {
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
            if e.is_cancelled() {
                push_agent_message(AgentMessage::assistant(
                    "Investigation cancelled.".to_string(),
                ));
            } else {
                push_agent_message(AgentMessage::error(e.display_message()));
            }
        }
    }
}
