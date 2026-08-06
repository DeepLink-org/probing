//! Investigation skill runner used by the Next Investigate workspace.

use std::collections::HashMap;

use dioxus::prelude::*;

use crate::agent::{
    evaluate_rules_for_skill, format_findings, load_skill, resolve_skill_id, run_skill,
    select_skill, skill_store_loaded, summarize_run,
};
use crate::api::ApiClient;
use crate::components::agent::step_outcome_to_card;
use crate::state::agent::{push_agent_message, AgentMessage};
use crate::state::llm_config::{LlmConfig, LLM_CONFIG};
use crate::state::page_context::PAGE_CONTEXT;
use crate::state::ui_tasks::{ui_agent_busy, UiTaskKind, UiTaskSession};

/// Submit a user message to the agent (opens flow / skills / LLM).
pub fn submit_agent_text(text: String) {
    if text.trim().is_empty() || ui_agent_busy() {
        return;
    }
    dispatch_agent_message(text);
}

/// Run a skill immediately (used from chips and source bridges).
pub fn trigger_skill(skill_id: String) {
    if ui_agent_busy() {
        return;
    }
    spawn_run_skill(skill_id, HashMap::new(), None);
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
