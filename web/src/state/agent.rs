//! Investigation Agent panel state.

use dioxus::prelude::*;
use probing_proto::prelude::DataFrame;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentStepStatus {
    Ok,
    Warn,
    Skipped,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentStepKind {
    Sql,
    Api,
    Navigate,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AgentStepCardData {
    pub step_id: String,
    pub title: String,
    pub kind: AgentStepKind,
    pub status: AgentStepStatus,
    pub body_text: String,
    pub dataframe: Option<DataFrame>,
    pub row_count: Option<usize>,
    pub navigate_view: Option<String>,
    pub api_path: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AgentMessageKind {
    User,
    Assistant,
    PlaybookRun,
    StepCard,
    Error,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AgentMessage {
    pub kind: AgentMessageKind,
    pub text: String,
    pub title: Option<String>,
    pub playbook_id: Option<String>,
    pub playbook_category: Option<String>,
    pub step: Option<AgentStepCardData>,
}

impl AgentMessage {
    pub fn user(text: String) -> Self {
        Self {
            kind: AgentMessageKind::User,
            text,
            title: None,
            playbook_id: None,
            playbook_category: None,
            step: None,
        }
    }

    pub fn assistant(text: String) -> Self {
        Self {
            kind: AgentMessageKind::Assistant,
            text,
            title: None,
            playbook_id: None,
            playbook_category: None,
            step: None,
        }
    }

    pub fn error(text: String) -> Self {
        Self {
            kind: AgentMessageKind::Error,
            text,
            title: None,
            playbook_id: None,
            playbook_category: None,
            step: None,
        }
    }

    pub fn playbook_run(
        playbook_id: String,
        title: String,
        category: String,
        docs: String,
    ) -> Self {
        Self {
            kind: AgentMessageKind::PlaybookRun,
            text: docs,
            title: Some(title),
            playbook_id: Some(playbook_id),
            playbook_category: Some(category),
            step: None,
        }
    }

    pub fn step_card(step: AgentStepCardData) -> Self {
        Self {
            kind: AgentMessageKind::StepCard,
            text: String::new(),
            title: None,
            playbook_id: None,
            playbook_category: None,
            step: Some(step),
        }
    }
}

pub static AGENT_PANEL_OPEN: GlobalSignal<bool> = Signal::global(|| false);
pub static AGENT_INPUT: GlobalSignal<String> = Signal::global(String::new);
pub static AGENT_MESSAGES: GlobalSignal<Vec<AgentMessage>> = Signal::global(Vec::new);
pub static AGENT_RUNNING: GlobalSignal<bool> = Signal::global(|| false);

pub fn push_agent_message(msg: AgentMessage) {
    AGENT_MESSAGES.write().push(msg);
}

pub fn clear_agent_messages() {
    AGENT_MESSAGES.write().clear();
}
