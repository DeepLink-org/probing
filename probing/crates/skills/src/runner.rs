//! Execute skill steps via a pluggable HTTP/query backend.

use std::collections::HashMap;
use std::fmt;

use probing_proto::prelude::DataFrame;

use crate::backend::{cluster_meta_note, SkillBackend};
use crate::interpret::{evaluate_rules, InterpretFinding, StepEvidence};
use crate::loader::{
    build_context, default_parameters, expand_template, load_skill, Skill, SkillStep,
};

#[derive(Debug, Clone)]
pub struct SkillRunError(pub String);

impl fmt::Display for SkillRunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SkillRunError {}

impl From<anyhow::Error> for SkillRunError {
    fn from(value: anyhow::Error) -> Self {
        Self(value.to_string())
    }
}

pub type Result<T> = std::result::Result<T, SkillRunError>;

#[derive(Debug, Clone, Default)]
pub struct RunOptions {
    pub include_ui: bool,
}

#[derive(Debug, Clone)]
pub enum StepOutcome {
    Sql {
        step_id: String,
        title: String,
        dataframe: DataFrame,
        row_count: usize,
        note: Option<String>,
        degraded: bool,
        cluster_meta: Option<crate::backend::ClusterQueryMeta>,
    },
    ApiText {
        step_id: String,
        title: String,
        text: String,
    },
    UiNavigate {
        step_id: String,
        title: String,
        view: String,
    },
    Skipped {
        step_id: String,
        title: String,
        reason: String,
    },
    Error {
        step_id: String,
        title: String,
        message: String,
    },
}

#[derive(Debug, Clone)]
pub struct RunResult {
    pub skill: Skill,
    pub parameters: HashMap<String, String>,
    pub context: HashMap<String, String>,
    pub outcomes: Vec<StepOutcome>,
    pub findings: Vec<InterpretFinding>,
    pub summary: String,
    pub had_error: bool,
    pub had_degraded: bool,
}

pub fn list_skills_catalog() -> Result<()> {
    use crate::loader::list_skill_ids;
    for id in list_skill_ids() {
        let _ = load_skill(&id)?;
    }
    Ok(())
}

pub fn plan_skill(skill_id: &str, overrides: HashMap<String, String>) -> Result<serde_json::Value> {
    let pb = load_skill(skill_id).map_err(|e| SkillRunError(e.to_string()))?;
    let mut params = default_parameters(&pb);
    params.extend(overrides);
    let ctx = build_context(&pb, &params);
    let steps: Vec<serde_json::Value> = pb
        .steps
        .iter()
        .map(|step| step_plan_json(step, &ctx))
        .collect();
    Ok(serde_json::json!({
        "skill_id": pb.id,
        "title": pb.title,
        "parameters": params,
        "steps": steps,
        "next_steps": pb.next_steps,
    }))
}

fn step_plan_json(step: &SkillStep, ctx: &HashMap<String, String>) -> serde_json::Value {
    let mut item = serde_json::json!({
        "id": step.id,
        "title": step.title,
        "type": step.step_type,
        "on_empty": step.on_empty,
    });
    if let Some(sql) = &step.sql {
        item["sql"] = serde_json::Value::String(expand_template(sql, ctx));
    }
    if let Some(path) = &step.path {
        item["path"] = serde_json::Value::String(expand_template(path, ctx));
    }
    if let Some(when) = &step.when {
        item["when"] = serde_json::Value::String(when.clone());
    }
    item
}

pub async fn resolve_use_global<B: SkillBackend>(
    backend: &B,
    pb: &Skill,
    overrides: &mut HashMap<String, String>,
) {
    if overrides.contains_key("use_global") {
        return;
    }
    let default = pb
        .parameters
        .iter()
        .find(|p| p.name == "use_global")
        .and_then(|p| match &p.default {
            serde_yaml::Value::Bool(b) => Some(*b),
            _ => None,
        })
        .unwrap_or(false);
    let peers = backend.peer_count().await;
    let use_global = peers > 0 && default;
    overrides.insert("use_global".to_string(), use_global.to_string());
}

pub async fn execute_skill<B: SkillBackend>(
    backend: &B,
    skill_id: &str,
    mut overrides: HashMap<String, String>,
    options: RunOptions,
) -> Result<RunResult> {
    let pb = load_skill(skill_id).map_err(|e| SkillRunError(e.to_string()))?;
    resolve_use_global(backend, &pb, &mut overrides).await;
    let ctx = build_context(&pb, &overrides);
    let mut outcomes = Vec::new();
    let mut evidence = Vec::new();
    let mut abort = false;

    for step in &pb.steps {
        if abort {
            break;
        }
        let outcome = run_step(backend, step, &ctx, &options).await;
        if let Some(ev) = outcome_to_evidence(&outcome) {
            evidence.push(ev);
        }
        if matches!(
            &outcome,
            StepOutcome::Error { .. } if step.on_empty == "abort"
        ) {
            abort = true;
        }
        outcomes.push(outcome);
    }

    let mut findings = evaluate_rules(&pb.interpretation, &evidence, &ctx);
    findings.extend(cluster_integrity_findings(&outcomes));

    let mut summary_ctx = ctx.clone();
    for ev in &evidence {
        summary_ctx.insert(
            format!("{}.row_count", ev.step_id),
            ev.row_count.to_string(),
        );
    }
    let summary = if pb.summary_template.is_empty() {
        String::new()
    } else {
        expand_template(&pb.summary_template, &summary_ctx)
    };

    let had_error = outcomes
        .iter()
        .any(|o| matches!(o, StepOutcome::Error { .. }));
    let had_degraded = outcomes.iter().any(|o| match o {
        StepOutcome::Sql { degraded, .. } => *degraded,
        _ => false,
    });

    Ok(RunResult {
        skill: pb,
        parameters: overrides,
        context: ctx,
        outcomes,
        findings,
        summary,
        had_error,
        had_degraded,
    })
}

pub async fn run_step<B: SkillBackend>(
    backend: &B,
    step: &SkillStep,
    ctx: &HashMap<String, String>,
    options: &RunOptions,
) -> StepOutcome {
    if let Some(reason) = should_skip_step(step, ctx) {
        return StepOutcome::Skipped {
            step_id: step.id.clone(),
            title: step.title.clone(),
            reason,
        };
    }
    match step.step_type.as_str() {
        "sql" => {
            let Some(sql_tpl) = &step.sql else {
                return StepOutcome::Error {
                    step_id: step.id.clone(),
                    title: step.title.clone(),
                    message: "SQL step missing query".to_string(),
                };
            };
            let sql = expand_template(sql_tpl, ctx);
            run_sql_step(backend, step, &sql).await
        }
        "api" => run_api_step(backend, step).await,
        "ui" if options.include_ui => StepOutcome::UiNavigate {
            step_id: step.id.clone(),
            title: step.title.clone(),
            view: step.view.clone().unwrap_or_else(|| "analytics".to_string()),
        },
        "ui" => StepOutcome::Skipped {
            step_id: step.id.clone(),
            title: step.title.clone(),
            reason: format!(
                "ui step (view={}) — run probing Web UI for navigation",
                step.view.as_deref().unwrap_or("?")
            ),
        },
        "config" => StepOutcome::Skipped {
            step_id: step.id.clone(),
            title: step.title.clone(),
            reason: "config steps are not applied automatically in CLI skill".to_string(),
        },
        other => StepOutcome::Skipped {
            step_id: step.id.clone(),
            title: step.title.clone(),
            reason: format!("unsupported step type: {other}"),
        },
    }
}

fn should_skip_step(step: &SkillStep, ctx: &HashMap<String, String>) -> Option<String> {
    let Some(when) = &step.when else {
        return None;
    };
    let w = when.trim();
    if w == "always" {
        return None;
    }
    if w == "{use_global}" || w.contains("use_global") {
        let use_global = ctx.get("use_global").map(|v| v == "true").unwrap_or(false);
        if !use_global {
            return Some("skipped (standalone / use_global=false)".to_string());
        }
    }
    None
}

fn sql_needs_cluster(sql: &str, step_cluster: bool) -> bool {
    step_cluster || sql.to_lowercase().contains("global.")
}

fn ensure_read_only_sql(sql: &str) -> Result<()> {
    let upper = sql.trim().to_uppercase();
    if upper.starts_with("SELECT")
        || upper.starts_with("WITH")
        || upper.starts_with("SHOW")
        || upper.starts_with("DESCRIBE")
    {
        return Ok(());
    }
    Err(SkillRunError(
        "Only read-only SQL is allowed in skills".to_string(),
    ))
}

fn dataframe_rows(df: &DataFrame) -> usize {
    df.cols.iter().map(|c| c.len()).max().unwrap_or(0)
}

async fn run_sql_step<B: SkillBackend>(backend: &B, step: &SkillStep, sql: &str) -> StepOutcome {
    if let Err(e) = ensure_read_only_sql(sql) {
        return StepOutcome::Error {
            step_id: step.id.clone(),
            title: step.title.clone(),
            message: e.0,
        };
    }
    let cluster = sql_needs_cluster(sql, step.cluster.unwrap_or(false));
    let result = if cluster {
        backend.cluster_query(sql).await.map(|(df, meta)| {
            let note = meta.as_ref().map(cluster_meta_note);
            (df, note, meta)
        })
    } else {
        backend.query_local(sql).await.map(|df| (df, None, None))
    };

    match result {
        Ok((df, note, cluster_meta)) => {
            let degraded = cluster_meta.as_ref().is_some_and(|m| m.partial);
            let rows = dataframe_rows(&df);
            if rows == 0 {
                match step.on_empty.as_str() {
                    "abort" => StepOutcome::Error {
                        step_id: step.id.clone(),
                        title: step.title.clone(),
                        message: step
                            .empty_message
                            .clone()
                            .unwrap_or_else(|| "Query returned no rows".to_string()),
                    },
                    "warn" => StepOutcome::Sql {
                        step_id: step.id.clone(),
                        title: step.title.clone(),
                        dataframe: df,
                        row_count: 0,
                        note,
                        degraded,
                        cluster_meta,
                    },
                    _ => StepOutcome::Skipped {
                        step_id: step.id.clone(),
                        title: step.title.clone(),
                        reason: step
                            .empty_message
                            .clone()
                            .unwrap_or_else(|| "No data".to_string()),
                    },
                }
            } else {
                StepOutcome::Sql {
                    step_id: step.id.clone(),
                    title: step.title.clone(),
                    dataframe: df,
                    row_count: rows,
                    note,
                    degraded,
                    cluster_meta,
                }
            }
        }
        Err(e) => {
            if step.on_empty == "skip" {
                StepOutcome::Skipped {
                    step_id: step.id.clone(),
                    title: step.title.clone(),
                    reason: e.0,
                }
            } else {
                StepOutcome::Error {
                    step_id: step.id.clone(),
                    title: step.title.clone(),
                    message: e.0,
                }
            }
        }
    }
}

async fn run_api_step<B: SkillBackend>(backend: &B, step: &SkillStep) -> StepOutcome {
    let path = step.path.clone().unwrap_or_default();
    match backend.get(&path).await {
        Ok(body) => StepOutcome::ApiText {
            step_id: step.id.clone(),
            title: step.title.clone(),
            text: body,
        },
        Err(e) => StepOutcome::Error {
            step_id: step.id.clone(),
            title: step.title.clone(),
            message: e.0,
        },
    }
}

fn outcome_to_evidence(outcome: &StepOutcome) -> Option<StepEvidence> {
    match outcome {
        StepOutcome::Sql {
            step_id,
            dataframe,
            row_count,
            ..
        } => Some(StepEvidence {
            step_id: step_id.clone(),
            row_count: *row_count,
            dataframe: dataframe.clone(),
        }),
        _ => None,
    }
}

fn cluster_integrity_findings(outcomes: &[StepOutcome]) -> Vec<InterpretFinding> {
    let mut findings = Vec::new();
    for outcome in outcomes {
        let StepOutcome::Sql {
            step_id,
            cluster_meta: Some(meta),
            ..
        } = outcome
        else {
            continue;
        };
        if !meta.partial {
            continue;
        }
        findings.push(InterpretFinding {
            rule_id: format!("{step_id}_partial_fanout"),
            severity: "error".to_string(),
            message: format!(
                "Cluster fan-out incomplete for step '{step_id}': {} nodes queried, {} failed, {} peer batches dropped — do not treat results as complete",
                meta.nodes_queried,
                meta.nodes_failed.len(),
                meta.peer_batches_dropped,
            ),
        });
    }
    findings
}

pub fn outcome_to_json(outcome: &StepOutcome) -> serde_json::Value {
    match outcome {
        StepOutcome::Sql {
            step_id,
            title,
            dataframe,
            row_count,
            note,
            degraded,
            cluster_meta,
        } => serde_json::json!({
            "step_id": step_id,
            "title": title,
            "status": if *degraded { "degraded" } else { "ok" },
            "row_count": row_count,
            "note": note,
            "cluster_meta": cluster_meta,
            "dataframe": dataframe,
        }),
        StepOutcome::ApiText {
            step_id,
            title,
            text,
        } => serde_json::json!({
            "step_id": step_id,
            "title": title,
            "status": "ok",
            "text": text,
        }),
        StepOutcome::UiNavigate {
            step_id,
            title,
            view,
        } => serde_json::json!({
            "step_id": step_id,
            "title": title,
            "status": "ui",
            "view": view,
        }),
        StepOutcome::Skipped {
            step_id,
            title,
            reason,
        } => serde_json::json!({
            "step_id": step_id,
            "title": title,
            "status": "skipped",
            "reason": reason,
        }),
        StepOutcome::Error {
            step_id,
            title,
            message,
        } => serde_json::json!({
            "step_id": step_id,
            "title": title,
            "status": "error",
            "message": message,
        }),
    }
}

pub fn run_result_to_json(result: &RunResult) -> serde_json::Value {
    let steps_out: Vec<serde_json::Value> = result.outcomes.iter().map(outcome_to_json).collect();
    let findings_json: Vec<serde_json::Value> = result
        .findings
        .iter()
        .map(|f| {
            serde_json::json!({
                "rule_id": f.rule_id,
                "severity": f.severity,
                "message": f.message,
            })
        })
        .collect();
    serde_json::json!({
        "skill_id": result.skill.id,
        "title": result.skill.title,
        "parameters": result.parameters,
        "steps": steps_out,
        "findings": findings_json,
        "summary": result.summary,
        "next_steps": result.skill.next_steps,
        "status": if result.had_error {
            "error"
        } else if result.had_degraded {
            "degraded"
        } else {
            "ok"
        },
        "data_quality": {
            "partial_cluster_fanout": result.had_degraded,
        },
    })
}
