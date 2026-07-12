//! Execute skill steps via the shared Rust runner (no Python).

use std::collections::HashMap;

use probing_skills::{build_context, resolve_use_global, run_step, RunOptions, Skill};

use crate::agent::skill::load_skill;
use crate::agent::skills_backend::WebBackend;
use crate::state::ui_tasks::{open_ui_task, UiTaskKind, UiTaskSession};
use crate::utils::error::{AppError, Result};

#[derive(Debug, Clone)]
pub enum StepOutcome {
    Sql {
        step_id: String,
        title: String,
        dataframe: probing_proto::prelude::DataFrame,
        row_count: usize,
        empty_message: Option<String>,
        cluster_note: Option<String>,
        cluster_meta: Option<probing_skills::backend::ClusterQueryMeta>,
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

fn map_outcome(
    outcome: probing_skills::runner::StepOutcome,
    path_hint: Option<String>,
) -> StepOutcome {
    use probing_skills::runner::StepOutcome as O;
    match outcome {
        O::Sql {
            step_id,
            title,
            dataframe,
            row_count,
            note,
            cluster_meta,
            ..
        } => StepOutcome::Sql {
            step_id,
            title,
            dataframe,
            row_count,
            empty_message: None,
            cluster_note: note,
            cluster_meta,
        },
        O::ApiText {
            step_id,
            title,
            text,
        } => StepOutcome::ApiText {
            step_id,
            title,
            text,
            path: path_hint,
        },
        O::UiNavigate {
            step_id,
            title,
            view,
        } => StepOutcome::UiNavigate {
            step_id,
            title,
            view,
        },
        O::Skipped {
            step_id,
            title,
            reason,
        } => StepOutcome::Skipped {
            step_id,
            title,
            reason,
        },
        O::Error {
            step_id,
            title,
            message,
        } => StepOutcome::Error {
            step_id,
            title,
            message,
        },
    }
}

pub async fn run_skill(
    skill_id: &str,
    mut overrides: HashMap<String, String>,
    session: Option<&UiTaskSession>,
) -> Result<(Skill, Vec<StepOutcome>, HashMap<String, String>)> {
    if session.is_some_and(|s| s.is_cancelled()) {
        return Err(AppError::Cancelled);
    }
    let skill =
        load_skill(skill_id).ok_or_else(|| AppError::Api(format!("Unknown skill: {skill_id}")))?;
    let backend = WebBackend;

    resolve_use_global(&backend, &skill, &mut overrides).await;
    if session.is_some_and(|s| s.is_cancelled()) {
        return Err(AppError::Cancelled);
    }
    let ctx = build_context(&skill, &overrides);
    let options = RunOptions { include_ui: true };

    let mut outcomes = Vec::new();
    for step in &skill.steps {
        if session.is_some_and(|s| s.is_cancelled()) {
            return Err(AppError::Cancelled);
        }
        let task = match session {
            Some(s) => s.open(
                UiTaskKind::Skill,
                step.title.clone(),
                Some(format!("{skill_id} · {}", step.id)),
            ),
            None => open_ui_task(
                UiTaskKind::Skill,
                step.title.clone(),
                Some(format!("{skill_id} · {}", step.id)),
            ),
        };
        let path_hint = step.path.clone();
        let raw = run_step(&backend, step, &ctx, &options).await;
        let outcome = map_outcome(raw, path_hint);
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
    Ok((skill, outcomes, ctx))
}
