//! OpenAI-compatible chat via `async-openai` (browser BYOK from localStorage).

use std::collections::HashMap;

use async_openai::{
    config::OpenAIConfig,
    error::OpenAIError,
    types::chat::{
        ChatCompletionRequestMessage, ChatCompletionRequestSystemMessageArgs,
        ChatCompletionRequestUserMessageArgs, CreateChatCompletionRequestArgs, ResponseFormat,
    },
    Client,
};
use dioxus::prelude::ReadableExt;
use serde::Deserialize;

use crate::agent::cluster::cluster_context_for_llm;
use crate::agent::runner::StepOutcome;
use crate::agent::{fetch_cluster_snapshot, list_skill_ids, load_skill, routing_context_for_llm};
use crate::state::llm_config::LlmConfig;
use crate::state::page_context::PAGE_CONTEXT;
use crate::utils::error::{AppError, Result};

#[derive(Debug, Deserialize)]
pub struct SkillSelection {
    pub skill_id: Option<String>,
    #[serde(default)]
    pub parameters: HashMap<String, String>,
    #[serde(default)]
    pub reply: String,
}

fn llm_client(config: &LlmConfig) -> Client<OpenAIConfig> {
    let api_base = config.api_base.trim().trim_end_matches('/');
    let openai_config = OpenAIConfig::new()
        .with_api_base(api_base)
        .with_api_key(config.api_key.trim());
    Client::with_config(openai_config)
}

fn map_openai_error(err: OpenAIError) -> AppError {
    AppError::Api(err.to_string())
}

fn skill_catalog_prompt() -> String {
    let mut lines = vec![routing_context_for_llm(), String::new()];
    lines.push("Skill details:".to_string());
    for id in list_skill_ids() {
        if let Some(pb) = load_skill(&id) {
            lines.push(format!(
                "- {}: {} — {}",
                pb.id,
                pb.title,
                pb.docs.lines().next().unwrap_or("").trim()
            ));
        }
    }
    lines.join("\n")
}

fn system_prompt_select() -> String {
    format!(
        "You are the Probing Investigate assistant for live AI training diagnostics \
         (skill-driven diagnostic agent).\n\
         Pick exactly ONE skill id from the catalog, or null if none apply.\n\
         Use the current page context and page snapshot to choose relevant skills and parameters.\n\
         Respond with JSON only (no markdown), shape:\n\
         {{\"skill_id\":\"slow_rank\"|null,\"parameters\":{{\"step_window\":\"20\"}},\"reply\":\"one sentence\"}}\n\
         parameters values must be strings. Allowed keys depend on skill (e.g. step_window, use_global, sample_limit).\n\
         For distributed training: prefer skills slow_rank, comm_bottleneck when cluster has peers; set use_global=true to fan-out via global.* tables.\n\
         Catalog:\n{}",
        skill_catalog_prompt()
    )
}

async fn workspace_context_block_with_cluster() -> String {
    let page = PAGE_CONTEXT.read().llm_block();
    let cluster = fetch_cluster_snapshot().await;
    format!("{page}\n\n{}", cluster_context_for_llm(&cluster))
}

fn extract_json_object(text: &str) -> &str {
    let trimmed = text.trim();
    if trimmed.starts_with('{') {
        return trimmed;
    }
    if let Some(start) = trimmed.find("```") {
        let rest = &trimmed[start + 3..];
        let rest = rest.strip_prefix("json").unwrap_or(rest);
        if let Some(end) = rest.find("```") {
            return rest[..end].trim();
        }
    }
    trimmed
}

async fn chat_completion(
    config: &LlmConfig,
    system: &str,
    user: &str,
    temperature: f32,
    json_mode: bool,
) -> Result<String> {
    let client = llm_client(config);

    let system_msg: ChatCompletionRequestMessage =
        ChatCompletionRequestSystemMessageArgs::default()
            .content(system)
            .build()
            .map_err(|e| AppError::Api(e.to_string()))?
            .into();

    let user_msg: ChatCompletionRequestMessage = ChatCompletionRequestUserMessageArgs::default()
        .content(user)
        .build()
        .map_err(|e| AppError::Api(e.to_string()))?
        .into();

    let mut builder = CreateChatCompletionRequestArgs::default();
    builder
        .model(config.model.as_str())
        .messages(vec![system_msg, user_msg])
        .temperature(temperature);

    if json_mode {
        builder.response_format(ResponseFormat::JsonObject);
    }

    let request = builder.build().map_err(|e| AppError::Api(e.to_string()))?;

    let response = client
        .chat()
        .create(request)
        .await
        .map_err(map_openai_error)?;

    response
        .choices
        .into_iter()
        .next()
        .and_then(|c| c.message.content)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| AppError::Api("LLM returned empty response".to_string()))
}

pub async fn select_skill(config: &LlmConfig, user_message: &str) -> Result<SkillSelection> {
    let page_block = workspace_context_block_with_cluster().await;

    let text = chat_completion(
        config,
        &format!("{}\n\n{}", system_prompt_select(), page_block),
        user_message,
        0.1,
        true,
    )
    .await?;

    let json_str = extract_json_object(&text);
    serde_json::from_str(json_str)
        .map_err(|e| AppError::Api(format!("LLM returned invalid JSON: {e}\nRaw: {text}")))
}

pub async fn summarize_run(
    config: &LlmConfig,
    user_message: &str,
    skill_id: &str,
    evidence: &str,
) -> Result<String> {
    let pb_title = load_skill(skill_id)
        .map(|p| p.title)
        .unwrap_or_else(|| skill_id.to_string());

    let system = "You summarize probing diagnostic results for an ML engineer. \
         Be concise (3-6 bullets). Cite specific numbers from evidence. \
         State uncertainty when data is missing. Use the same language as the user.";

    let page_block = workspace_context_block_with_cluster().await;

    let user = format!(
        "User question: {user_message}\n\
         Skill: {pb_title}\n\
         Workspace:\n{page_block}\n\
         Evidence:\n{evidence}\n\
         Summarize findings and suggest next actions.",
    );

    chat_completion(config, system, &user, 0.3, false).await
}

pub fn outcomes_to_evidence(outcomes: &[StepOutcome]) -> String {
    let mut parts = Vec::new();
    for o in outcomes {
        match o {
            StepOutcome::Sql {
                title,
                row_count,
                empty_message,
                cluster_note,
                ..
            } => {
                let mut line = if *row_count > 0 {
                    format!("[{title}] {row_count} rows returned")
                } else if let Some(msg) = empty_message {
                    format!("[{title}] empty — {msg}")
                } else {
                    format!("[{title}] no rows")
                };
                if let Some(note) = cluster_note {
                    line.push_str(&format!(" ({note})"));
                }
                parts.push(line);
            }
            StepOutcome::ApiText { title, text, .. } => {
                let preview: String = text.lines().take(12).collect::<Vec<_>>().join("\n");
                parts.push(format!("[{title}]\n{preview}"));
            }
            StepOutcome::Skipped { title, reason, .. } => {
                parts.push(format!("[{title}] skipped: {reason}"));
            }
            StepOutcome::Error { title, message, .. } => {
                parts.push(format!("[{title}] ERROR: {message}"));
            }
            StepOutcome::UiNavigate { title, view, .. } => {
                parts.push(format!("[{title}] navigate to {view}"));
            }
        }
    }
    parts.join("\n\n")
}
