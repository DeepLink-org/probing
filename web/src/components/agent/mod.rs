//! Investigation Agent — playbook-driven diagnostic chat panel.

pub mod chat;
pub mod panel;
mod settings;
mod step_card;
mod view_route;

pub use panel::AgentPanel;
pub use settings::LlmSettingsOverlay;
