//! Diagnostic agent: playbook loading, matching, and step execution.

mod llm;
mod playbook;
mod runner;

pub use llm::{outcomes_to_evidence, select_playbook, summarize_run, PlaybookSelection};

pub use playbook::{
    default_parameters, derive_variables, expand_sql, list_playbook_ids, load_playbook,
    match_playbooks, resolve_playbook_id, Playbook, PlaybookStep,
};
pub use runner::{run_playbook, StepOutcome};
