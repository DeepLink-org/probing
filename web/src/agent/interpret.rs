//! Thin wrappers around the shared ``probing-skills`` interpreter.

use std::collections::HashMap;

use probing_proto::prelude::DataFrame;
use probing_skills::{
    build_summary, cluster_integrity_findings, evaluate_rules as shared_evaluate_rules,
    InterpretFinding, InterpretRule, Skill,
};

use crate::agent::runner::StepOutcome;

#[derive(Debug, Clone)]
pub struct StepEvidence {
    pub step_id: String,
    pub row_count: usize,
    pub dataframe: DataFrame,
}

pub fn evidence_from_outcomes(outcomes: &[StepOutcome]) -> Vec<StepEvidence> {
    outcomes
        .iter()
        .filter_map(|o| match o {
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
        })
        .collect()
}

pub fn evaluate_rules(
    rules: &[InterpretRule],
    steps: &[StepEvidence],
    params: &HashMap<String, String>,
) -> Vec<InterpretFinding> {
    let shared_steps: Vec<probing_skills::StepEvidence> = steps
        .iter()
        .map(|s| probing_skills::StepEvidence {
            step_id: s.step_id.clone(),
            row_count: s.row_count,
            dataframe: s.dataframe.clone(),
        })
        .collect();
    shared_evaluate_rules(rules, &shared_steps, params)
}

pub fn evaluate_rules_for_skill(
    skill: &Skill,
    outcomes: &[StepOutcome],
    params: &HashMap<String, String>,
) -> Vec<InterpretFinding> {
    let steps = evidence_from_outcomes(outcomes);
    let mut findings = evaluate_rules(&skill.interpretation, &steps, params);
    let runner_outcomes: Vec<probing_skills::runner::StepOutcome> =
        outcomes.iter().filter_map(web_outcome_to_runner).collect();
    findings.extend(cluster_integrity_findings(&runner_outcomes));
    findings
}

pub fn build_skill_summary(
    skill: &Skill,
    outcomes: &[StepOutcome],
    params: &HashMap<String, String>,
) -> String {
    let steps: Vec<probing_skills::StepEvidence> = evidence_from_outcomes(outcomes)
        .into_iter()
        .map(|s| probing_skills::StepEvidence {
            step_id: s.step_id,
            row_count: s.row_count,
            dataframe: s.dataframe,
        })
        .collect();
    build_summary(skill, &steps, params)
}

fn web_outcome_to_runner(outcome: &StepOutcome) -> Option<probing_skills::runner::StepOutcome> {
    match outcome {
        StepOutcome::Sql {
            step_id,
            title,
            dataframe,
            row_count,
            cluster_meta,
            ..
        } => Some(probing_skills::runner::StepOutcome::Sql {
            step_id: step_id.clone(),
            title: title.clone(),
            dataframe: dataframe.clone(),
            row_count: *row_count,
            note: None,
            degraded: cluster_meta.as_ref().is_some_and(|m| m.partial),
            cluster_meta: cluster_meta.clone(),
        }),
        _ => None,
    }
}

pub fn format_findings(findings: &[InterpretFinding]) -> String {
    if findings.is_empty() {
        return String::new();
    }
    let mut lines = vec!["### Interpretation".to_string()];
    for f in findings {
        lines.push(format!(
            "- **[{}]** {} — {}",
            f.severity.to_uppercase(),
            f.rule_id,
            f.message
        ));
    }
    lines.join("\n")
}
