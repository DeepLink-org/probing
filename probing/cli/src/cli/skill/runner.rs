//! Execute skill steps against a probing endpoint (sql / api / config).

use std::collections::HashMap;

use anyhow::{anyhow, Result};
use probing_proto::prelude::{DataFrame, Query};

use crate::cli::ctrl::ProbeEndpoint;
use crate::table::{render, OutputFormat};

use super::interpret::{evaluate_rules, InterpretFinding, StepEvidence};
use super::loader::{build_context, expand_template, load_skill, Skill, SkillStep};

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ClusterQueryMeta {
    pub partial: bool,
    pub nodes_queried: usize,
    pub nodes_failed: Vec<String>,
    pub peer_batches_dropped: usize,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum StepOutcome {
    Sql {
        step_id: String,
        title: String,
        dataframe: DataFrame,
        row_count: usize,
        note: Option<String>,
        degraded: bool,
        cluster_meta: Option<ClusterQueryMeta>,
    },
    ApiText {
        step_id: String,
        title: String,
        text: String,
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

fn dataframe_rows(df: &DataFrame) -> usize {
    df.cols.iter().map(|c| c.len()).max().unwrap_or(0)
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
    Err(anyhow!("Only read-only SQL is allowed in skills"))
}

async fn cluster_query(
    ctrl: &ProbeEndpoint,
    expr: &str,
) -> Result<(DataFrame, Option<ClusterQueryMeta>)> {
    let body = serde_json::json!({
        "expr": expr,
        "cluster": true,
    });
    let reply = ctrl
        .post_json("/apis/cluster/query", &body.to_string())
        .await?;
    let value: serde_json::Value = serde_json::from_str(&reply)?;
    if let Some(err) = value.get("error").and_then(|v| v.as_str()) {
        anyhow::bail!("{err}");
    }
    let df = value
        .get("dataframe")
        .ok_or_else(|| anyhow!("missing dataframe in cluster response"))?;
    let dataframe: DataFrame = serde_json::from_value(df.clone())?;
    let cluster_meta = value.get("meta").map(parse_cluster_meta);
    Ok((dataframe, cluster_meta))
}

fn parse_cluster_meta(meta: &serde_json::Value) -> ClusterQueryMeta {
    let nodes_failed: Vec<String> = meta
        .get("nodes_failed")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let peer_batches_dropped = meta
        .get("peer_batches_dropped")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;
    let nodes_queried = meta
        .get("nodes_queried")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;
    let partial = meta
        .get("partial")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || !nodes_failed.is_empty()
        || peer_batches_dropped > 0;
    ClusterQueryMeta {
        partial,
        nodes_queried,
        nodes_failed,
        peer_batches_dropped,
    }
}

fn cluster_meta_note(meta: &ClusterQueryMeta) -> String {
    format!(
        "cluster fan-out · {} nodes queried · {} failed · {} peer batches dropped{}",
        meta.nodes_queried,
        meta.nodes_failed.len(),
        meta.peer_batches_dropped,
        if meta.partial { " · PARTIAL" } else { "" }
    )
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

async fn fetch_cluster_peer_count(ctrl: &ProbeEndpoint) -> usize {
    match ctrl.get("/apis/nodes?limit=1024").await {
        Ok(reply) => match serde_json::from_str::<probing_proto::prelude::NodeListResponse>(&reply)
        {
            Ok(resp) => resp.total.saturating_sub(1),
            Err(_) => 0,
        },
        Err(_) => 0,
    }
}

async fn resolve_use_global(
    ctrl: &ProbeEndpoint,
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
    let peers = fetch_cluster_peer_count(ctrl).await;
    let use_global = peers > 0 && default;
    overrides.insert("use_global".to_string(), use_global.to_string());
}

async fn run_sql_step(ctrl: &ProbeEndpoint, step: &SkillStep, sql: &str) -> StepOutcome {
    if let Err(e) = ensure_read_only_sql(sql) {
        return StepOutcome::Error {
            step_id: step.id.clone(),
            title: step.title.clone(),
            message: e.to_string(),
        };
    }
    let cluster = sql_needs_cluster(sql, step.cluster.unwrap_or(false));
    let result = if cluster {
        cluster_query(ctrl, sql).await.map(|(df, meta)| {
            let note = meta.as_ref().map(cluster_meta_note);
            (df, note, meta)
        })
    } else {
        ctrl.query(Query::new(sql.to_string()))
            .await
            .map(|df| (df, None, None))
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
                    reason: e.to_string(),
                }
            } else {
                StepOutcome::Error {
                    step_id: step.id.clone(),
                    title: step.title.clone(),
                    message: e.to_string(),
                }
            }
        }
    }
}

async fn run_api_step(ctrl: &ProbeEndpoint, step: &SkillStep) -> StepOutcome {
    let path = step.path.clone().unwrap_or_default();
    match ctrl.get(&path).await {
        Ok(body) => StepOutcome::ApiText {
            step_id: step.id.clone(),
            title: step.title.clone(),
            text: body,
        },
        Err(e) => StepOutcome::Error {
            step_id: step.id.clone(),
            title: step.title.clone(),
            message: e.to_string(),
        },
    }
}

async fn run_step(
    ctrl: &ProbeEndpoint,
    step: &SkillStep,
    ctx: &HashMap<String, String>,
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
            run_sql_step(ctrl, step, &sql).await
        }
        "api" => run_api_step(ctrl, step).await,
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

fn print_outcome(outcome: &StepOutcome, format: OutputFormat) {
    match outcome {
        StepOutcome::Sql {
            title,
            dataframe,
            row_count,
            note,
            degraded,
            ..
        } => {
            let tag = if *degraded { " [DEGRADED]" } else { "" };
            println!("\n## {title}{tag} ({row_count} rows)");
            if let Some(n) = note {
                eprintln!("({n})");
            }
            if *row_count > 0 {
                render(dataframe, format);
            }
        }
        StepOutcome::ApiText { title, text, .. } => {
            println!("\n## {title}");
            println!("{text}");
        }
        StepOutcome::Skipped { title, reason, .. } => {
            eprintln!("\n## {title} [skipped]");
            eprintln!("{reason}");
        }
        StepOutcome::Error { title, message, .. } => {
            eprintln!("\n## {title} [error]");
            eprintln!("{message}");
        }
    }
}

fn print_findings(findings: &[InterpretFinding]) {
    if findings.is_empty() {
        return;
    }
    println!("\n### Interpretation");
    for f in findings {
        println!(
            "[{}] {} — {}",
            f.severity.to_uppercase(),
            f.rule_id,
            f.message
        );
    }
}

pub fn list_skills() -> Result<()> {
    use super::loader::list_skill_ids;
    println!("Available diagnostic skills:\n");
    for id in list_skill_ids() {
        let pb = load_skill(&id)?;
        println!("  {:<22} {:<12} {}", id, pb.category, pb.title);
    }
    Ok(())
}

pub async fn run_skill(
    ctrl: ProbeEndpoint,
    skill_id: &str,
    mut overrides: HashMap<String, String>,
    format: OutputFormat,
) -> Result<()> {
    let pb = load_skill(skill_id)?;
    resolve_use_global(&ctrl, &pb, &mut overrides).await;
    let ctx = build_context(&pb, &overrides);

    println!("# {} ({})", pb.title, pb.id);
    if !pb.docs.is_empty() {
        println!("{}\n", pb.docs);
    }

    let mut outcomes = Vec::new();
    let mut evidence = Vec::new();
    let mut abort = false;

    for step in &pb.steps {
        if abort {
            break;
        }
        let outcome = run_step(&ctrl, step, &ctx).await;
        if let Some(ev) = outcome_to_evidence(&outcome) {
            evidence.push(ev);
        }
        print_outcome(&outcome, format);
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
    print_findings(&findings);

    if !pb.summary_template.is_empty() {
        let mut summary_ctx = ctx.clone();
        for ev in &evidence {
            summary_ctx.insert(
                format!("{}.row_count", ev.step_id),
                ev.row_count.to_string(),
            );
        }
        println!("\n{}", expand_template(&pb.summary_template, &summary_ctx));
    }

    if !pb.next_steps.is_empty() {
        println!("\n### Next steps");
        for line in &pb.next_steps {
            println!("- {line}");
        }
    }

    let had_error = outcomes
        .iter()
        .any(|o| matches!(o, StepOutcome::Error { .. }));
    let had_degraded = outcomes.iter().any(|o| match o {
        StepOutcome::Sql { degraded, .. } => *degraded,
        _ => false,
    });
    if had_error {
        anyhow::bail!("skill finished with errors");
    }
    if had_degraded {
        anyhow::bail!(
            "skill finished with degraded/partial cluster data — results must not be treated as complete"
        );
    }
    Ok(())
}

/// Execute a skill and return structured JSON for MCP / agent consumers.
pub async fn run_skill_json(
    ctrl: ProbeEndpoint,
    skill_id: &str,
    mut overrides: HashMap<String, String>,
) -> Result<serde_json::Value> {
    let pb = load_skill(skill_id)?;
    resolve_use_global(&ctrl, &pb, &mut overrides).await;
    let ctx = build_context(&pb, &overrides);

    let mut steps_out = Vec::new();
    let mut outcomes = Vec::new();
    let mut evidence = Vec::new();
    let mut abort = false;

    for step in &pb.steps {
        if abort {
            break;
        }
        let outcome = run_step(&ctrl, step, &ctx).await;
        if let Some(ev) = outcome_to_evidence(&outcome) {
            evidence.push(ev);
        }
        steps_out.push(outcome_to_json(&outcome));
        outcomes.push(outcome);
        if matches!(
            outcomes.last(),
            Some(StepOutcome::Error { .. }) if step.on_empty == "abort"
        ) {
            abort = true;
        }
    }

    let mut findings = evaluate_rules(&pb.interpretation, &evidence, &ctx);
    findings.extend(cluster_integrity_findings(&outcomes));
    let findings_json: Vec<serde_json::Value> = findings
        .iter()
        .map(|f| {
            serde_json::json!({
                "rule_id": f.rule_id,
                "severity": f.severity,
                "message": f.message,
            })
        })
        .collect();

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

    let had_error = steps_out.iter().any(|s| {
        s.get("status")
            .and_then(|v| v.as_str())
            .is_some_and(|st| st == "error")
    });
    let had_degraded = steps_out.iter().any(|s| {
        s.get("status")
            .and_then(|v| v.as_str())
            .is_some_and(|st| st == "degraded")
    });

    let payload = serde_json::json!({
        "skill_id": pb.id,
        "title": pb.title,
        "parameters": overrides,
        "steps": steps_out,
        "findings": findings_json,
        "summary": summary,
        "next_steps": pb.next_steps,
        "status": if had_error {
            "error"
        } else if had_degraded {
            "degraded"
        } else {
            "ok"
        },
        "data_quality": {
            "partial_cluster_fanout": had_degraded,
        },
    });

    if had_error {
        anyhow::bail!(payload.to_string());
    }
    Ok(payload)
}

fn outcome_to_json(outcome: &StepOutcome) -> serde_json::Value {
    match outcome {
        StepOutcome::Sql {
            step_id,
            title,
            dataframe,
            row_count,
            note,
            degraded,
            cluster_meta,
        } => {
            serde_json::json!({
                "step_id": step_id,
                "title": title,
                "status": if *degraded { "degraded" } else { "ok" },
                "row_count": row_count,
                "note": note,
                "cluster_meta": cluster_meta,
                "dataframe": dataframe,
            })
        }
        StepOutcome::ApiText {
            step_id,
            title,
            text,
        } => {
            serde_json::json!({
                "step_id": step_id,
                "title": title,
                "status": "ok",
                "text": text,
            })
        }
        StepOutcome::Skipped {
            step_id,
            title,
            reason,
        } => {
            serde_json::json!({
                "step_id": step_id,
                "title": title,
                "status": "skipped",
                "reason": reason,
            })
        }
        StepOutcome::Error {
            step_id,
            title,
            message,
        } => {
            serde_json::json!({
                "step_id": step_id,
                "title": title,
                "status": "error",
                "message": message,
            })
        }
    }
}

#[cfg(test)]
mod cluster_meta_tests {
    use super::parse_cluster_meta;

    #[test]
    fn partial_when_nodes_failed_even_without_flag() {
        let meta = parse_cluster_meta(&serde_json::json!({
            "partial": false,
            "nodes_queried": 8,
            "nodes_failed": ["10.0.0.3:8080"],
            "peer_batches_dropped": 0,
        }));
        assert!(meta.partial);
        assert_eq!(meta.nodes_failed.len(), 1);
    }
}
