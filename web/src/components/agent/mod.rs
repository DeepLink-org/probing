//! Investigation Agent — playbook-driven diagnostic chat panel.

mod panel;
mod settings;
mod step_card;
mod view_route;

pub use panel::AgentPanel;
pub use settings::LlmSettingsOverlay;
pub use step_card::{step_outcome_to_card, AgentPlaybookRunCard, AgentStepCard};
pub use view_route::{agent_view_label, agent_view_to_route, navigate_to_agent_view};
