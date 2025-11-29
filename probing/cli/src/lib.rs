pub mod cli;
pub mod table;

#[cfg(target_os = "linux")]
pub mod inject;

use anyhow::Result;
use clap::Parser;
use env_logger::Env;

const ENV_PROBING_LOGLEVEL: &str = "PROBING_LOGLEVEL";

/// Main entry point for the CLI, can be called from Python or as a binary
#[tokio::main]
pub async fn cli_main(args: Vec<String>) -> Result<()> {
    // Try to initialize logger, but ignore error if already initialized
    // (e.g., when called from Python after library is loaded via #[ctor])
    let _ = env_logger::try_init_from_env(Env::new().filter(ENV_PROBING_LOGLEVEL));
    cli::Cli::parse_from(args).run().await
}
