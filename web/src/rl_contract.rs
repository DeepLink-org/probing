//! Framework-neutral RL span classification rules.
//!
//! Any RL framework can drive the Probing UI by attaching standard span attributes
//! (see `probing.rl.STANDARD_ATTRS` in the Python SDK). The frontend only interprets
//! these generic fields — never framework-specific names.

use crate::api::SpanInfo;

pub fn attr_string(attributes: &Option<String>, key: &str) -> Option<String> {
    let attrs = attributes.as_ref()?;
    let value = serde_json::from_str::<serde_json::Value>(attrs).ok()?;
    let raw = value.get(key)?;
    match raw {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// Rollout/sample timeline requires trajectory or sample identity on each span.
pub fn has_trajectory_identity(attributes: &Option<String>) -> bool {
    attr_string(attributes, "trajectory_id").is_some() || attr_string(attributes, "sample_id").is_some()
}

/// Training timeline spans must be tagged as train phases and keyed by step/batch.
pub fn is_train_phase_span(
    name: &str,
    phase: &str,
    kind: Option<&str>,
    attributes: &Option<String>,
) -> bool {
    if phase.starts_with("train") || name.starts_with("train.") {
        return true;
    }
    if attr_string(attributes, "actor_role").as_deref() == Some("trainer") {
        return true;
    }
    matches!(kind, Some("rl.train") | Some("train.step") | Some("rl.phase"))
        && (phase.contains("train") || name.contains("train"))
}

pub fn is_train_timeline_span(
    name: &str,
    phase: &str,
    kind: Option<&str>,
    attributes: &Option<String>,
) -> bool {
    if attr_string(attributes, "train_step_id").is_none() && attr_string(attributes, "batch_id").is_none()
    {
        return false;
    }
    is_train_phase_span(name, phase, kind, attributes)
}

/// Parent span that groups a distributed rollout/train step across processes.
pub fn is_step_parent_span(name: &str, kind: Option<&str>) -> bool {
    name == "rollout.step"
        || name == "train.step"
        || name == "rollout.submit"
        || name == "rollout.submit_next"
        || matches!(kind, Some("rl.step") | Some("train.step"))
}

pub fn is_rollout_submit_parent_span(name: &str) -> bool {
    name == "rollout.submit" || name == "rollout.submit_next"
}

/// Worker process that executes rollout logic (framework sets `actor_role` or `process_role`).
pub fn is_rollout_worker_role(role: &str) -> bool {
    role == "rollout"
        || role == "rollout_actor"
        || role.ends_with("_rollout")
        || role.contains("rollout_worker")
}

pub fn is_cross_process_child_candidate(span: &SpanInfo) -> bool {
    if logical_step_key(span).is_none() || is_cross_process_parent_span(span) {
        return false;
    }

    let role = attr_string(&span.attributes, "process_role")
        .or_else(|| attr_string(&span.attributes, "actor_role"))
        .unwrap_or_default();
    is_rollout_worker_role(&role) || span.name.starts_with("custom.generate")
}

pub fn is_cross_process_parent_span(span: &SpanInfo) -> bool {
    is_step_parent_span(&span.name, span.phase.as_deref())
}

pub fn logical_step_key(span: &SpanInfo) -> Option<LogicalStepKey> {
    let rollout_id = attr_string(&span.attributes, "rollout_id")
        .or_else(|| attr_string(&span.attributes, "step_id"))?;
    let step_id = attr_string(&span.attributes, "step_id")
        .or_else(|| attr_string(&span.attributes, "train_step_id"))?;

    if rollout_id == "-1" || step_id == "-1" {
        return None;
    }

    Some(LogicalStepKey { rollout_id, step_id })
}

#[derive(Clone, Eq, PartialEq)]
pub struct LogicalStepKey {
    pub rollout_id: String,
    pub step_id: String,
}

/// User-facing hint when train timeline has no matching spans.
pub const TRAIN_TIMELINE_EMPTY_HINT: &str =
    "No train batch spans found. Tag spans with rollout_id, train_step_id or batch_id, and phase=train.* via probing.rl.";

/// User-facing hint when rollout timeline has no matching spans.
pub const ROLLOUT_TIMELINE_EMPTY_HINT: &str =
    "No rollout trajectory spans found. Tag spans with rollout_id plus trajectory_id or sample_id via probing.rl.";
