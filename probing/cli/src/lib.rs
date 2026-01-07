pub mod cli;
pub mod table;

#[cfg(target_os = "linux")]
pub mod inject;

use anyhow::Result;
use clap::Parser;
use env_logger::Env;
use std::time::Duration;
use tokio::time::timeout;

const ENV_PROBING_LOGLEVEL: &str = "PROBING_LOGLEVEL";

/// Main entry point for the CLI, can be called from Python or as a binary
pub async fn cli_main(args: Vec<String>) -> Result<()> {
    let _ = env_logger::try_init_from_env(Env::new().filter(ENV_PROBING_LOGLEVEL));
    match timeout(Duration::from_secs(10), cli::Cli::parse_from(args).run()).await {
        Ok(result) => result,
        Err(_) => Err(anyhow::anyhow!("CLI command timed out")),
    }
}
