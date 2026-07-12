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
    overrides: HashMap<String, String>,
    options: RunOptions,
) -> Result<RunResult> {
    let pb = load_skill(skill_id).map_err(|e| SkillRunError(e.to_string()))?;
    execute_skill_pb(backend, pb, overrides, options).await
}

/// Run a loaded skill (shared by [`execute_skill`] and unit tests).
pub async fn execute_skill_pb<B: SkillBackend>(
    backend: &B,
    pb: Skill,
    mut overrides: HashMap<String, String>,
    options: RunOptions,
) -> Result<RunResult> {
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

    let summary = build_summary(&pb, &evidence, &ctx);

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
            let rows = df.row_count();
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
        Ok(body) => {
            if body.trim().is_empty() && step.on_empty == "warn" {
                StepOutcome::ApiText {
                    step_id: step.id.clone(),
                    title: step.title.clone(),
                    text: "(empty response — step may be unavailable)".to_string(),
                }
            } else {
                StepOutcome::ApiText {
                    step_id: step.id.clone(),
                    title: step.title.clone(),
                    text: body,
                }
            }
        }
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

pub fn build_summary(
    skill: &Skill,
    evidence: &[StepEvidence],
    ctx: &HashMap<String, String>,
) -> String {
    if skill.summary_template.is_empty() {
        return String::new();
    }
    let mut summary_ctx = ctx.clone();
    for ev in evidence {
        summary_ctx.insert(
            format!("{}.row_count", ev.step_id),
            ev.row_count.to_string(),
        );
        for col in &ev.dataframe.names {
            let key = format!("{}.{}", ev.step_id, col);
            if let Some(val) = crate::interpret::cell_display_first(&ev.dataframe, col) {
                summary_ctx.insert(key, val);
            }
        }
    }
    expand_template(&skill.summary_template, &summary_ctx)
}

/// Surface incomplete cluster fan-out as an explicit finding (shared by CLI and Web).
pub fn cluster_integrity_findings(outcomes: &[StepOutcome]) -> Vec<InterpretFinding> {
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use async_trait::async_trait;
    use probing_proto::prelude::{DataFrame, Seq};

    use super::*;
    use crate::backend::ClusterQueryMeta;
    use crate::loader::{InterpretRule, Skill, SkillParameter, SkillStep};

    struct MockBackend {
        peers: usize,
        local_rows: usize,
        cluster_partial: bool,
        calls: Arc<AtomicUsize>,
    }

    impl MockBackend {
        fn new(peers: usize, local_rows: usize) -> Self {
            Self {
                peers,
                local_rows,
                cluster_partial: false,
                calls: Arc::new(AtomicUsize::new(0)),
            }
        }

        fn with_partial(mut self) -> Self {
            self.cluster_partial = true;
            self
        }

        fn df(rows: usize) -> DataFrame {
            if rows == 0 {
                return DataFrame::default();
            }
            DataFrame::new(
                vec!["x".into()],
                vec![Seq::SeqF64((0..rows as i64).map(|i| i as f64).collect())],
            )
        }
    }

    #[async_trait]
    impl SkillBackend for MockBackend {
        async fn query_local(&self, sql: &str) -> Result<DataFrame> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            if sql.contains("FAIL") {
                return Err(SkillRunError("query failed".into()));
            }
            Ok(Self::df(self.local_rows))
        }

        async fn cluster_query(&self, sql: &str) -> Result<(DataFrame, Option<ClusterQueryMeta>)> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            let meta = ClusterQueryMeta {
                partial: self.cluster_partial,
                nodes_queried: 4,
                nodes_failed: if self.cluster_partial {
                    vec!["rank-3".into()]
                } else {
                    vec![]
                },
                peer_batches_dropped: if self.cluster_partial { 1 } else { 0 },
            };
            let _ = sql;
            Ok((Self::df(self.local_rows), Some(meta)))
        }

        async fn get(&self, path: &str) -> Result<String> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            Ok(format!("body:{path}"))
        }

        async fn peer_count(&self) -> usize {
            self.peers
        }
    }

    fn sample_step(id: &str, sql: &str) -> SkillStep {
        SkillStep {
            id: id.into(),
            title: format!("title-{id}"),
            step_type: "sql".into(),
            sql: Some(sql.into()),
            path: None,
            view: None,
            on_empty: "skip".into(),
            empty_message: None,
            when: None,
            cluster: None,
        }
    }

    fn sample_skill(steps: Vec<SkillStep>, interpretation: Vec<InterpretRule>) -> Skill {
        Skill {
            id: "test_skill".into(),
            title: "Test".into(),
            category: String::new(),
            docs: String::new(),
            tags: vec![],
            keywords: vec![],
            trigger_keywords: Default::default(),
            parameters: vec![SkillParameter {
                name: "use_global".into(),
                default: serde_yaml::Value::Bool(true),
            }],
            steps,
            interpretation,
            summary_template: "rows={available_tables.row_count}".into(),
            next_steps: vec![],
            variables: HashMap::new(),
        }
    }

    #[tokio::test]
    async fn run_step_sql_returns_rows() {
        let backend = MockBackend::new(0, 3);
        let step = sample_step("q", "SELECT 1");
        let outcome = run_step(&backend, &step, &HashMap::new(), &RunOptions::default()).await;
        match outcome {
            StepOutcome::Sql { row_count, .. } => assert_eq!(row_count, 3),
            other => panic!("expected Sql, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn run_step_when_use_global_skips_without_flag() {
        let backend = MockBackend::new(0, 1);
        let mut step = sample_step("g", "SELECT 1 FROM global.nccl.coll_perf");
        step.when = Some("{use_global}".into());
        let ctx = HashMap::from([("use_global".into(), "false".into())]);
        let outcome = run_step(&backend, &step, &ctx, &RunOptions::default()).await;
        assert!(matches!(outcome, StepOutcome::Skipped { .. }));
        assert_eq!(backend.calls.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn run_step_on_empty_abort_returns_error() {
        let backend = MockBackend::new(0, 0);
        let mut step = sample_step("empty", "SELECT 1");
        step.on_empty = "abort".into();
        step.empty_message = Some("no data".into());
        let outcome = run_step(&backend, &step, &HashMap::new(), &RunOptions::default()).await;
        match outcome {
            StepOutcome::Error { message, .. } => assert_eq!(message, "no data"),
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn run_step_cluster_partial_sets_degraded() {
        let backend = MockBackend::new(2, 2).with_partial();
        let mut step = sample_step("cluster", "SELECT 1 FROM global.nccl.coll_perf");
        step.cluster = Some(true);
        let outcome = run_step(&backend, &step, &HashMap::new(), &RunOptions::default()).await;
        match outcome {
            StepOutcome::Sql { degraded, note, .. } => {
                assert!(degraded);
                assert!(note.as_ref().is_some_and(|n| n.contains("PARTIAL")));
            }
            other => panic!("expected Sql, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn run_step_rejects_mutating_sql() {
        let backend = MockBackend::new(0, 1);
        let step = sample_step("bad", "SET probing.x=1");
        let outcome = run_step(&backend, &step, &HashMap::new(), &RunOptions::default()).await;
        assert!(matches!(outcome, StepOutcome::Error { .. }));
    }

    #[tokio::test]
    async fn resolve_use_global_honors_override() {
        let backend = MockBackend::new(4, 1);
        let pb = sample_skill(vec![], vec![]);
        let mut overrides = HashMap::from([("use_global".into(), "false".into())]);
        resolve_use_global(&backend, &pb, &mut overrides).await;
        assert_eq!(
            overrides.get("use_global").map(String::as_str),
            Some("false")
        );
    }

    #[tokio::test]
    async fn resolve_use_global_auto_when_peers_and_default_true() {
        let backend = MockBackend::new(3, 1);
        let pb = sample_skill(vec![], vec![]);
        let mut overrides = HashMap::new();
        resolve_use_global(&backend, &pb, &mut overrides).await;
        assert_eq!(
            overrides.get("use_global").map(String::as_str),
            Some("true")
        );
    }

    #[tokio::test]
    async fn execute_skill_pb_abort_stops_after_on_empty_abort() {
        let backend = MockBackend::new(0, 0);
        let mut first = sample_step("a", "SELECT 1");
        first.on_empty = "abort".into();
        let second = sample_step("b", "SELECT 2");
        let pb = sample_skill(vec![first, second], vec![]);
        let result = execute_skill_pb(&backend, pb, HashMap::new(), RunOptions::default())
            .await
            .unwrap();
        assert!(result.had_error);
        assert_eq!(result.outcomes.len(), 1, "second step must not run");
    }

    #[tokio::test]
    async fn execute_skill_pb_interpretation_and_summary() {
        let backend = MockBackend::new(0, 0);
        let mut step = sample_step("available_tables", "SELECT 1");
        step.on_empty = "warn".into();
        let rules = vec![InterpretRule {
            id: "no_tables".into(),
            when: "step:available_tables | rows == 0".into(),
            severity: "error".into(),
            message: "no tables".into(),
        }];
        let pb = sample_skill(vec![step], rules);
        let result = execute_skill_pb(&backend, pb, HashMap::new(), RunOptions::default())
            .await
            .unwrap();
        assert_eq!(result.findings.len(), 1);
        assert_eq!(result.findings[0].rule_id, "no_tables");
        assert!(result.summary.contains("rows=0"));
    }

    #[tokio::test]
    async fn execute_skill_pb_had_degraded_on_partial_fanout() {
        let backend = MockBackend::new(0, 2).with_partial();
        let mut step = sample_step("cluster", "SELECT 1 FROM global.nccl.coll_perf");
        step.cluster = Some(true);
        let pb = sample_skill(vec![step], vec![]);
        let result = execute_skill_pb(&backend, pb, HashMap::new(), RunOptions::default())
            .await
            .unwrap();
        assert!(result.had_degraded);
        assert!(result
            .findings
            .iter()
            .any(|f| f.rule_id == "cluster_partial_fanout"));
        let json = run_result_to_json(&result);
        assert_eq!(json["status"], "degraded");
        assert_eq!(json["data_quality"]["partial_cluster_fanout"], true);
    }

    #[test]
    fn outcome_to_json_maps_statuses() {
        let skipped = outcome_to_json(&StepOutcome::Skipped {
            step_id: "s".into(),
            title: "t".into(),
            reason: "r".into(),
        });
        assert_eq!(skipped["status"], "skipped");
        let err = outcome_to_json(&StepOutcome::Error {
            step_id: "e".into(),
            title: "t".into(),
            message: "m".into(),
        });
        assert_eq!(err["status"], "error");
    }
}
