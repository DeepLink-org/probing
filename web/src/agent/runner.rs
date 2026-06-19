//! Execute playbook steps against the probing HTTP API.

use std::collections::HashMap;

use crate::agent::playbook::{build_context, expand_sql, load_playbook, Playbook, PlaybookStep};
use crate::api::ApiClient;
use crate::utils::error::Result;
use probing_proto::prelude::DataFrame;

#[derive(Debug, Clone)]
pub enum StepOutcome {
    Sql {
        step_id: String,
        title: String,
        dataframe: DataFrame,
        row_count: usize,
        empty_message: Option<String>,
    },
    ApiText {
        step_id: String,
        title: String,
        text: String,
        path: Option<String>,
    },
    UiNavigate {
        step_id: String,
        title: String,
        view: String,
    },
    Skipped {
        step_id: String,
        title: String,
        reason: String,
    },
    Error {
        step_id: String,
        title: String,
        message: String,
    },
}

fn dataframe_rows(df: &DataFrame) -> usize {
    df.cols.iter().map(|c| c.len()).max().unwrap_or(0)
}

fn should_skip_step(step: &PlaybookStep, ctx: &HashMap<String, String>) -> Option<String> {
    let Some(when) = &step.when else {
        return None;
    };
    let w = when.trim();
    if w == "always" {
        return None;
    }
    if w == "{use_global}" || w.contains("use_global") {
        let use_global = ctx.get("use_global").map(|v| v == "true").unwrap_or(false);
        if !use_global {
            return Some("skipped (use_global=false)".to_string());
        }
    }
    None
}

async fn run_sql_step(step: &PlaybookStep, sql: &str) -> StepOutcome {
    let client = ApiClient::new();
    match client.execute_query(sql).await {
        Ok(df) => {
            let rows = dataframe_rows(&df);
            if rows == 0 {
                match step.on_empty.as_str() {
                    "abort" => StepOutcome::Error {
                        step_id: step.id.clone(),
                        title: step.title.clone(),
                        message: step
                            .empty_message
                            .clone()
                            .unwrap_or_else(|| "Query returned no rows".to_string()),
                    },
                    "warn" => StepOutcome::Sql {
                        step_id: step.id.clone(),
                        title: step.title.clone(),
                        dataframe: df,
                        row_count: 0,
                        empty_message: step.empty_message.clone(),
                    },
                    _ => StepOutcome::Skipped {
                        step_id: step.id.clone(),
                        title: step.title.clone(),
                        reason: step
                            .empty_message
                            .clone()
                            .unwrap_or_else(|| "No data".to_string()),
                    },
                }
            } else {
                StepOutcome::Sql {
                    step_id: step.id.clone(),
                    title: step.title.clone(),
                    dataframe: df,
                    row_count: rows,
                    empty_message: None,
                }
            }
        }
        Err(e) => {
            if step.on_empty == "skip" {
                StepOutcome::Skipped {
                    step_id: step.id.clone(),
                    title: step.title.clone(),
                    reason: e.display_message(),
                }
            } else {
                StepOutcome::Error {
                    step_id: step.id.clone(),
                    title: step.title.clone(),
                    message: e.display_message(),
                }
            }
        }
    }
}

async fn run_api_step(step: &PlaybookStep) -> StepOutcome {
    let path = step.path.clone().unwrap_or_default();
    let client = ApiClient::new();
    if path.contains("callstack") {
        match client.get_callstack_with_mode(None, "mixed").await {
            Ok(frames) => {
                let text = frames
                    .iter()
                    .take(24)
                    .map(|f| format!("{f}"))
                    .collect::<Vec<_>>()
                    .join("\n");
                StepOutcome::ApiText {
                    step_id: step.id.clone(),
                    title: step.title.clone(),
                    text,
                    path: Some(path),
                }
            }
            Err(e) => StepOutcome::Error {
                step_id: step.id.clone(),
                title: step.title.clone(),
                message: e.display_message(),
            },
        }
    } else {
        match client.get_raw(&path).await {
            Ok(body) => StepOutcome::ApiText {
                step_id: step.id.clone(),
                title: step.title.clone(),
                text: body,
                path: Some(path),
            },
            Err(e) => StepOutcome::Error {
                step_id: step.id.clone(),
                title: step.title.clone(),
                message: e.display_message(),
            },
        }
    }
}

async fn run_step(step: &PlaybookStep, ctx: &HashMap<String, String>) -> StepOutcome {
    if let Some(reason) = should_skip_step(step, ctx) {
        return StepOutcome::Skipped {
            step_id: step.id.clone(),
            title: step.title.clone(),
            reason,
        };
    }
    match step.step_type.as_str() {
        "sql" => {
            let Some(sql_tpl) = &step.sql else {
                return StepOutcome::Error {
                    step_id: step.id.clone(),
                    title: step.title.clone(),
                    message: "SQL step missing query".to_string(),
                };
            };
            let sql = expand_sql(sql_tpl, ctx);
            run_sql_step(step, &sql).await
        }
        "api" => run_api_step(step).await,
        "ui" => StepOutcome::UiNavigate {
            step_id: step.id.clone(),
            title: step.title.clone(),
            view: step.view.clone().unwrap_or_else(|| "analytics".to_string()),
        },
        other => StepOutcome::Skipped {
            step_id: step.id.clone(),
            title: step.title.clone(),
            reason: format!("unsupported step type: {other}"),
        },
    }
}

pub async fn run_playbook(
    playbook_id: &str,
    overrides: HashMap<String, String>,
) -> Result<(Playbook, Vec<StepOutcome>)> {
    let pb = load_playbook(playbook_id)
        .ok_or_else(|| crate::utils::error::AppError::Api(format!("Unknown playbook: {playbook_id}")))?;
    let ctx = build_context(&pb, &overrides);
    let mut outcomes = Vec::new();
    for step in &pb.steps {
        let outcome = run_step(step, &ctx).await;
        let abort = matches!(
            outcome,
            StepOutcome::Error { .. } if step.on_empty == "abort"
        );
        outcomes.push(outcome);
        if abort {
            break;
        }
    }
    Ok((pb, outcomes))
}
