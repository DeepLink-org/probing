//! Structured diagnostic skills (``probing skill``).

mod backend;
mod runner;

pub use probing_skills::{
    build_context, default_parameters, derive_variables, evaluate_rules, expand_template,
    list_skill_ids, load_skill, plan_skill, InterpretFinding, InterpretRule, Skill, SkillParameter,
    SkillStep, StepEvidence,
};
pub use runner::{list_skills as list_skills_sync, run_skill, run_skill_json};

use std::collections::HashMap;

use anyhow::Result;
use clap::Subcommand;

use crate::cli::ctrl::ProbeEndpoint;
use crate::table::OutputFormat;

#[derive(Subcommand, Debug, Clone)]
pub enum SkillCommand {
    /// List available diagnostic skills
    List,
    /// Install skills into Cursor, Claude Code, and Codex skill directories
    Install {
        /// Re-sync from bundled source (same as ``update``)
        #[arg(long)]
        update: bool,
        /// Install to user dirs (~/.cursor/skills, ~/.claude/skills, ~/.agents/skills)
        #[arg(long)]
        user: bool,
        /// Comma-separated agents: cursor, claude, codex
        #[arg(long, value_delimiter = ',')]
        agent: Vec<String>,
        /// Install even when agent markers were not detected
        #[arg(long)]
        force: bool,
        /// Skill source directory (default: bundled skills)
        #[arg(long)]
        from: Option<std::path::PathBuf>,
    },
    /// Update installed skills from bundled source
    Update {
        #[arg(long)]
        user: bool,
        #[arg(long, value_delimiter = ',')]
        agent: Vec<String>,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        from: Option<std::path::PathBuf>,
    },
    /// Run a diagnostic skill against the target process
    Run {
        /// Skill id (e.g. health_overview, slow_rank)
        skill_id: String,

        /// Parameter override as key=value (repeatable)
        #[arg(short = 'p', long = "set", value_name = "KEY=VALUE")]
        params: Vec<String>,

        /// Force global.* cluster fan-out (overrides auto-detection)
        #[arg(long)]
        global: bool,

        /// Do not fan out global.* queries even when cluster peers exist
        #[arg(long)]
        local: bool,

        #[arg(short, long, value_enum, default_value_t = OutputFormat::Table)]
        format: OutputFormat,
    },
}

pub async fn run(ctrl: ProbeEndpoint, cmd: SkillCommand) -> Result<()> {
    match cmd {
        SkillCommand::List => list_skills_sync(),
        SkillCommand::Install {
            update,
            user,
            agent,
            force,
            from,
        } => run_python_install("install", update, user, &agent, force, from.as_deref()),
        SkillCommand::Update {
            user,
            agent,
            force,
            from,
        } => run_python_install("update", true, user, &agent, force, from.as_deref()),
        SkillCommand::Run {
            skill_id,
            params,
            global,
            local,
            format,
        } => {
            let mut overrides = parse_params(&params)?;
            if global {
                overrides.insert("use_global".to_string(), "true".to_string());
            } else if local {
                overrides.insert("use_global".to_string(), "false".to_string());
            }
            runner::run_skill(ctrl, &skill_id, overrides, format).await
        }
    }
}

fn parse_params(params: &[String]) -> Result<HashMap<String, String>> {
    let mut out = HashMap::new();
    for p in params {
        let Some((k, v)) = p.split_once('=') else {
            anyhow::bail!("invalid --set {p:?}, expected key=value");
        };
        out.insert(k.to_string(), v.to_string());
    }
    Ok(out)
}

fn run_python_install(
    action: &str,
    update: bool,
    user: bool,
    agents: &[String],
    force: bool,
    from: Option<&std::path::Path>,
) -> Result<()> {
    let python = std::env::var("PROBING_PYTHON").unwrap_or_else(|_| {
        if cfg!(windows) {
            "python".to_string()
        } else {
            "python3".to_string()
        }
    });
    let mut cmd = std::process::Command::new(python);
    cmd.arg("-m").arg("probing.skills").arg(action);
    if update {
        cmd.arg("--update");
    }
    if user {
        cmd.arg("--user");
    }
    if force {
        cmd.arg("--force");
    }
    if !agents.is_empty() {
        cmd.arg("--agent").arg(agents.join(","));
    }
    if let Some(path) = from {
        cmd.arg("--from").arg(path);
    }
    let status = cmd.status()?;
    if status.success() {
        Ok(())
    } else {
        anyhow::bail!("probing skill {action} failed (exit {status})");
    }
}
