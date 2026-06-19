//! Diagnostic agent: playbook loading, matching, and step execution.

mod cluster;
mod llm;
pub mod page_tools;
mod playbook;
mod routing;
mod runner;
mod source_bridge;

pub use cluster::fetch_cluster_snapshot;
pub use llm::{outcomes_to_evidence, select_playbook, summarize_run};
pub use page_tools::refresh_page_snapshot_for_route;
pub use playbook::{list_playbook_ids, load_playbook, resolve_playbook_id};
pub use routing::routing_context_for_llm;
pub use runner::{run_playbook, StepOutcome};
pub use source_bridge::{
    ask_agent_about_source, ask_and_run_agent_about_source, run_playbook_with_source,
    suggested_playbooks_for_source,
};
