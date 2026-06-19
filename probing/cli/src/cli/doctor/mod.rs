//! Structured diagnostic playbooks (``probing doctor``).

mod interpret;
mod loader;
mod runner;

pub use runner::{list_playbooks as list_playbooks_sync, run_doctor};

use std::collections::HashMap;

use anyhow::Result;
use clap::Subcommand;

use crate::cli::ctrl::ProbeEndpoint;
use crate::table::OutputFormat;

#[derive(Subcommand, Debug, Clone)]
pub enum DoctorCommand {
    /// List available diagnostic playbooks
    List,
    /// Run a diagnostic playbook against the target process
    Run {
        /// Playbook id (e.g. health_overview, slow_rank)
        playbook_id: String,

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

pub async fn run(ctrl: ProbeEndpoint, cmd: DoctorCommand) -> Result<()> {
    match cmd {
        DoctorCommand::List => list_playbooks_sync(),
        DoctorCommand::Run {
            playbook_id,
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
            runner::run_doctor(ctrl, &playbook_id, overrides, format).await
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
