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

use crate::agent::{list_playbook_ids, load_playbook};
use crate::agent::runner::StepOutcome;
use crate::state::investigation::INVESTIGATION_CONTEXT;
use crate::state::llm_config::LlmConfig;
use crate::utils::error::{AppError, Result};

#[derive(Debug, Deserialize)]
pub struct PlaybookSelection {
    pub playbook_id: Option<String>,
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

fn playbook_catalog_prompt() -> String {
    let mut lines = Vec::new();
    for id in list_playbook_ids() {
        if let Some(pb) = load_playbook(id) {
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
        "You are the Probing Investigation Agent for live AI training diagnostics.\n\
         Pick exactly ONE playbook id from the catalog, or null if none apply.\n\
         Respond with JSON only (no markdown), shape:\n\
         {{\"playbook_id\":\"slow_rank\"|null,\"parameters\":{{\"step_window\":\"20\"}},\"reply\":\"one sentence\"}}\n\
         parameters values must be strings. Allowed keys depend on playbook (e.g. step_window, use_global, sample_limit).\n\
         Catalog:\n{}",
        playbook_catalog_prompt()
    )
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

    let request = builder
        .build()
        .map_err(|e| AppError::Api(e.to_string()))?;

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

pub async fn select_playbook(config: &LlmConfig, user_message: &str) -> Result<PlaybookSelection> {
    let ctx = INVESTIGATION_CONTEXT.read().clone();
    let ctx_line = if ctx.is_empty() {
        String::new()
    } else {
        format!("\nInvestigation context: {}", ctx.summary())
    };

    let text = chat_completion(
        config,
        &system_prompt_select(),
        &format!("{user_message}{ctx_line}"),
        0.1,
        true,
    )
    .await?;

    let json_str = extract_json_object(&text);
    serde_json::from_str(json_str).map_err(|e| {
        AppError::Api(format!("LLM returned invalid JSON: {e}\nRaw: {text}"))
    })
}

pub async fn summarize_run(
    config: &LlmConfig,
    user_message: &str,
    playbook_id: &str,
    evidence: &str,
) -> Result<String> {
    let pb_title = load_playbook(playbook_id)
        .map(|p| p.title)
        .unwrap_or_else(|| playbook_id.to_string());

    let system = "You summarize probing diagnostic results for an ML engineer. \
         Be concise (3-6 bullets). Cite specific numbers from evidence. \
         State uncertainty when data is missing. Use the same language as the user.";

    let user = format!(
        "User question: {user_message}\n\
         Playbook: {pb_title}\n\
         Evidence:\n{evidence}\n\
         Summarize findings and suggest next actions."
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
                ..
            } => {
                if *row_count > 0 {
                    parts.push(format!("[{title}] {row_count} rows returned"));
                } else if let Some(msg) = empty_message {
                    parts.push(format!("[{title}] empty — {msg}"));
                } else {
                    parts.push(format!("[{title}] no rows"));
                }
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
