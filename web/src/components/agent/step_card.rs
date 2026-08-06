//! Convert skill step outcomes into Agent message cards.

use crate::agent::StepOutcome;
use crate::state::agent::{AgentStepCardData, AgentStepKind, AgentStepStatus};

pub fn step_outcome_to_card(outcome: StepOutcome) -> AgentStepCardData {
    match outcome {
        StepOutcome::Sql {
            step_id,
            title,
            dataframe,
            row_count,
            empty_message,
            cluster_note,
            cluster_meta,
            ..
        } => {
            let partial = cluster_meta.as_ref().is_some_and(|m| m.partial);
            let status = if partial {
                AgentStepStatus::Warn
            } else if row_count > 0 {
                AgentStepStatus::Ok
            } else if empty_message.is_some() {
                AgentStepStatus::Warn
            } else {
                AgentStepStatus::Skipped
            };
            let cluster_note = match (partial, cluster_note) {
                (true, Some(note)) => Some(format!("partial · {note}")),
                (true, None) => Some("partial cluster data".to_string()),
                (false, note) => note,
            };
            AgentStepCardData {
                step_id,
                title,
                kind: AgentStepKind::Sql,
                status,
                body_text: empty_message.unwrap_or_default(),
                dataframe: Some(dataframe),
                row_count: Some(row_count),
                navigate_view: None,
                api_path: None,
                cluster_note,
            }
        }
        StepOutcome::ApiText {
            step_id,
            title,
            text,
            path,
        } => AgentStepCardData {
            step_id,
            title,
            kind: AgentStepKind::Api,
            status: AgentStepStatus::Ok,
            body_text: text,
            dataframe: None,
            row_count: None,
            navigate_view: None,
            api_path: path,
            cluster_note: None,
        },
        StepOutcome::UiNavigate {
            step_id,
            title,
            view,
        } => AgentStepCardData {
            step_id,
            title,
            kind: AgentStepKind::Navigate,
            status: AgentStepStatus::Ok,
            body_text: String::new(),
            dataframe: None,
            row_count: None,
            navigate_view: Some(view),
            api_path: None,
            cluster_note: None,
        },
        StepOutcome::Skipped {
            step_id,
            title,
            reason,
        } => AgentStepCardData {
            step_id,
            title,
            kind: AgentStepKind::Sql,
            status: AgentStepStatus::Skipped,
            body_text: reason,
            dataframe: None,
            row_count: None,
            navigate_view: None,
            api_path: None,
            cluster_note: None,
        },
        StepOutcome::Error {
            step_id,
            title,
            message,
        } => AgentStepCardData {
            step_id,
            title,
            kind: AgentStepKind::Sql,
            status: AgentStepStatus::Error,
            body_text: message,
            dataframe: None,
            row_count: None,
            navigate_view: None,
            api_path: None,
            cluster_note: None,
        },
    }
}
