//! CLI wrappers around the shared ``probing-skills`` runner.

use std::collections::HashMap;

use anyhow::Result;
use probing_skills::runner::{
    execute_skill, run_result_to_json, RunOptions, RunResult, StepOutcome,
};

use crate::cli::ctrl::ProbeEndpoint;
use crate::table::{render, OutputFormat};

use super::backend::CliBackend;
use probing_skills::loader::{list_skill_ids, load_skill};

pub fn list_skills() -> Result<()> {
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
    overrides: HashMap<String, String>,
    format: OutputFormat,
) -> Result<()> {
    let result = execute_skill(
        &CliBackend(ctrl),
        skill_id,
        overrides,
        RunOptions::default(),
    )
    .await
    .map_err(|e| anyhow::anyhow!(e.0))?;
    print_run_result(&result, format)?;
    if result.had_error {
        anyhow::bail!("skill finished with errors");
    }
    if result.had_degraded {
        anyhow::bail!(
            "skill finished with degraded/partial cluster data — results must not be treated as complete"
        );
    }
    Ok(())
}

pub async fn run_skill_json(
    ctrl: ProbeEndpoint,
    skill_id: &str,
    overrides: HashMap<String, String>,
) -> Result<serde_json::Value> {
    let result = execute_skill(
        &CliBackend(ctrl),
        skill_id,
        overrides,
        RunOptions::default(),
    )
    .await
    .map_err(|e| anyhow::anyhow!(e.0))?;
    let payload = run_result_to_json(&result);
    if result.had_error {
        anyhow::bail!(payload.to_string());
    }
    Ok(payload)
}

fn print_run_result(result: &RunResult, format: OutputFormat) -> Result<()> {
    println!("# {} ({})", result.skill.title, result.skill.id);
    if !result.skill.docs.is_empty() {
        println!("{}\n", result.skill.docs);
    }
    for outcome in &result.outcomes {
        print_outcome(outcome, format);
    }
    if !result.findings.is_empty() {
        println!("\n### Interpretation");
        for f in &result.findings {
            println!(
                "[{}] {} — {}",
                f.severity.to_uppercase(),
                f.rule_id,
                f.message
            );
        }
    }
    if !result.summary.is_empty() {
        println!("\n{}", result.summary);
    }
    if !result.skill.next_steps.is_empty() {
        println!("\n### Next steps");
        for line in &result.skill.next_steps {
            println!("- {line}");
        }
    }
    Ok(())
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
        StepOutcome::UiNavigate { title, view, .. } => {
            eprintln!("\n## {title} [ui]");
            eprintln!("navigate to view: {view}");
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

#[cfg(test)]
mod tests {
    use super::print_outcome;
    use crate::table::OutputFormat;
    use probing_proto::prelude::DataFrame;
    use probing_skills::runner::StepOutcome;

    #[test]
    fn print_outcome_accepts_sql() {
        let outcome = StepOutcome::Sql {
            step_id: "s".into(),
            title: "Health".into(),
            dataframe: DataFrame::default(),
            row_count: 0,
            note: None,
            degraded: false,
            cluster_meta: None,
        };
        print_outcome(&outcome, OutputFormat::Table);
    }
}
