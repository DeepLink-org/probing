//! Diagnostic agent: skill loading, matching, and step execution.

mod cluster;
mod interpret;
mod llm;
pub mod page_tools;
mod routing;
mod runner;
mod skill;
mod source_bridge;

pub use cluster::fetch_cluster_snapshot;
pub use interpret::{evaluate_rules, evidence_from_outcomes, format_findings};
pub use llm::{outcomes_to_evidence, select_skill, summarize_run};
pub use page_tools::refresh_page_snapshot_for_route;
pub use routing::routing_context_for_llm;
pub use runner::{run_skill, StepOutcome};
pub use skill::{list_skill_ids, load_skill, resolve_skill_id};
pub use source_bridge::{
    ask_agent_about_source, ask_and_run_agent_about_source, run_skill_with_source,
    suggested_skills_for_source,
};
