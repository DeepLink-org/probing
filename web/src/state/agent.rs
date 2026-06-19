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
    /// e.g. "cluster fan-out · 4 nodes queried"
    pub cluster_note: Option<String>,
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

/// One-shot action consumed by [`AgentChat`] on mount/update (avoids component↔agent cycles).
#[derive(Clone, Debug, PartialEq)]
pub enum AgentPendingAction {
    SubmitText(String),
    RunPlaybook(String),
}

pub static AGENT_PENDING_ACTION: GlobalSignal<Option<AgentPendingAction>> = Signal::global(|| None);

pub static AGENT_ACTION_TICK: GlobalSignal<u32> = Signal::global(|| 0);

pub fn open_agent_prefill(text: String) {
    *AGENT_INPUT.write() = text;
    *AGENT_PANEL_OPEN.write() = true;
}

pub fn queue_agent_action(action: AgentPendingAction) {
    *AGENT_PENDING_ACTION.write() = Some(action);
    *AGENT_PANEL_OPEN.write() = true;
    *AGENT_ACTION_TICK.write() += 1;
}

pub fn take_agent_pending_action() -> Option<AgentPendingAction> {
    AGENT_PENDING_ACTION.write().take()
}

pub fn push_agent_message(msg: AgentMessage) {
    AGENT_MESSAGES.write().push(msg);
}

pub fn clear_agent_messages() {
    AGENT_MESSAGES.write().clear();
}

/// Agent side panel width relative to the main workspace (when open).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AgentPanelWidth {
    #[serde(rename = "third")]
    Third,
    #[serde(rename = "two_thirds")]
    TwoThirds,
}

impl Default for AgentPanelWidth {
    fn default() -> Self {
        Self::Third
    }
}

impl AgentPanelWidth {
    pub fn floating_class(&self) -> &'static str {
        match self {
            Self::Third => "w-[33vw] min-w-[320px] max-w-[480px]",
            Self::TwoThirds => "w-[66vw] min-w-[480px] max-w-[960px]",
        }
    }
}

const AGENT_WIDTH_KEY: &str = "probing_agent_panel_width";

pub static AGENT_PANEL_WIDTH: GlobalSignal<AgentPanelWidth> =
    Signal::global(AgentPanelWidth::default);

pub fn load_agent_panel_width() {
    let Some(window) = web_sys::window() else {
        return;
    };
    let Some(storage) = window.local_storage().ok().flatten() else {
        return;
    };
    let Ok(Some(raw)) = storage.get_item(AGENT_WIDTH_KEY) else {
        return;
    };
    if let Ok(w) = serde_json::from_str::<AgentPanelWidth>(&raw) {
        *AGENT_PANEL_WIDTH.write() = w;
    }
}

pub fn save_agent_panel_width(width: AgentPanelWidth) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let Some(storage) = window.local_storage().ok().flatten() else {
        return;
    };
    if let Ok(raw) = serde_json::to_string(&width) {
        let _ = storage.set_item(AGENT_WIDTH_KEY, &raw);
    }
    *AGENT_PANEL_WIDTH.write() = width;
}
