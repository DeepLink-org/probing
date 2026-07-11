//! Shared diagnostic skill runtime (loader, interpreter, step runner).

pub mod api;
pub mod backend;
pub mod catalog;
pub mod discovery;
pub mod interpret;
pub mod loader;
pub mod routing;
pub mod runner;

pub use api::{
    catalog_json, catalog_to_json, list_skills_json, load_skill_json, routing_json, skill_from_api,
    skill_to_json,
};
pub use backend::{ClusterQueryMeta, SkillBackend};
pub use catalog::{load_catalog, load_intents, load_pages, CatalogEntry};
pub use interpret::{evaluate_rules, InterpretFinding, StepEvidence};
pub use loader::{
    build_context, default_parameters, derive_variables, expand_template, list_skill_ids,
    load_skill, InterpretRule, KeywordsSpec, Skill, SkillParameter, SkillStep,
};
pub use routing::match_skills;
pub use runner::{
    execute_skill, plan_skill, resolve_use_global, run_step, RunOptions, RunResult, StepOutcome,
};
