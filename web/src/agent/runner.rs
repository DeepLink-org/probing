//! Execute playbook steps against the probing HTTP API.

use std::collections::HashMap;

use crate::agent::cluster::{
    default_use_global, execute_sql_for_agent, format_cluster_meta, sql_needs_cluster_fanout,
    fetch_cluster_snapshot,
};
use crate::agent::playbook::{build_context, expand_sql, load_playbook, Playbook, PlaybookStep};
use crate::api::ClusterQueryMeta;
use crate::state::ui_tasks::{open_ui_task, UiTaskKind, UiTaskSession};
use crate::utils::error::{AppError, Result};
use probing_proto::prelude::DataFrame;

#[derive(Debug, Clone)]
pub enum StepOutcome {
    Sql {
        step_id: String,
        title: String,
        dataframe: DataFrame,
        row_count: usize,
        empty_message: Option<String>,
        cluster_note: Option<String>,
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

fn playbook_default_use_global(pb: &Playbook) -> bool {
    pb.parameters
        .iter()
        .find(|p| p.name == "use_global")
        .and_then(|p| match &p.default {
            serde_yaml::Value::Bool(b) => Some(*b),
            _ => None,
        })
        .unwrap_or(false)
}

async fn resolve_overrides(
    pb: &Playbook,
    mut overrides: HashMap<String, String>,
) -> HashMap<String, String> {
    if overrides.contains_key("use_global") {
        return overrides;
    }
    let snapshot = fetch_cluster_snapshot().await;
    let default = default_use_global(&snapshot, playbook_default_use_global(pb));
    overrides.insert("use_global".to_string(), default.to_string());
    overrides
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
            return Some("skipped (单机模式 / use_global=false)".to_string());
        }
    }
    None
}

fn sql_outcome_from_df(
    step: &PlaybookStep,
    df: DataFrame,
    cluster_meta: Option<ClusterQueryMeta>,
) -> StepOutcome {
    let rows = dataframe_rows(&df);
    let cluster_note = cluster_meta.as_ref().map(format_cluster_meta);
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
                cluster_note,
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
            cluster_note,
        }
    }
}

async fn run_sql_step(step: &PlaybookStep, sql: &str) -> StepOutcome {
    let cluster_fanout = sql_needs_cluster_fanout(sql, step.cluster.unwrap_or(false));
    match execute_sql_for_agent(sql, cluster_fanout).await {
        Ok((df, meta)) => sql_outcome_from_df(step, df, meta),
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
    let client = crate::api::ApiClient::new();
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
    } else if path.contains("/apis/nodes") || path == "/apis/nodes" {
        match client.get_nodes().await {
            Ok(nodes) => {
                let text = nodes
                    .iter()
                    .map(|n| {
                        format!(
                            "rank={} host={} addr={} status={}",
                            n.rank
                                .map(|r| r.to_string())
                                .unwrap_or_else(|| "—".to_string()),
                            n.host,
                            n.addr,
                            n.status.clone().unwrap_or_else(|| "?".to_string())
                        )
                    })
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
    session: Option<&UiTaskSession>,
) -> Result<(Playbook, Vec<StepOutcome>)> {
    if session.is_some_and(|s| s.is_cancelled()) {
        return Err(AppError::Cancelled);
    }
    let pb = load_playbook(playbook_id)
        .ok_or_else(|| AppError::Api(format!("Unknown playbook: {playbook_id}")))?;
    let overrides = resolve_overrides(&pb, overrides).await;
    if session.is_some_and(|s| s.is_cancelled()) {
        return Err(AppError::Cancelled);
    }
    let ctx = build_context(&pb, &overrides);
    let mut outcomes = Vec::new();
    for step in &pb.steps {
        if session.is_some_and(|s| s.is_cancelled()) {
            return Err(AppError::Cancelled);
        }
        let task = match session {
            Some(s) => s.open(
                UiTaskKind::Playbook,
                step.title.clone(),
                Some(format!("{playbook_id} · {}", step.id)),
            ),
            None => open_ui_task(
                UiTaskKind::Playbook,
                step.title.clone(),
                Some(format!("{playbook_id} · {}", step.id)),
            ),
        };
        let outcome = run_step(step, &ctx).await;
        if task.is_cancelled() {
            task.cancel();
            return Err(AppError::Cancelled);
        }
        match &outcome {
            StepOutcome::Error { message, .. } => task.fail(message),
            _ => task.finish(),
        }
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
