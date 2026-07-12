pub mod cli;
pub mod table;

#[cfg(feature = "python-bridge")]
pub mod pyo3;

#[cfg(target_os = "linux")]
pub mod inject;

use anyhow::Result;
use clap::error::ErrorKind;
use clap::FromArgMatches;
use env_logger::Env;

const ENV_PROBING_LOGLEVEL: &str = "PROBING_LOGLEVEL";

fn is_help_or_version(err: &clap::Error) -> bool {
    matches!(
        err.kind(),
        ErrorKind::DisplayHelp
            | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
            | ErrorKind::DisplayVersion
    )
}

/// Parse argv; returns `None` after printing `--help` / `--version` (success, no further work).
fn parse_cli(args: Vec<String>) -> Result<Option<cli::Cli>> {
    let cmd = cli::Cli::build_command();
    match cmd.clone().try_get_matches_from(args) {
        Ok(matches) => {
            if matches.subcommand().is_none() {
                let mut help_cmd = cmd;
                help_cmd.print_long_help()?;
                return Ok(None);
            }
            Ok(Some(cli::Cli::from_arg_matches(&matches)?))
        }
        Err(e) if is_help_or_version(&e) => {
            e.print()?;
            Ok(None)
        }
        Err(e) => Err(e.into()),
    }
}

/// Main entry point for the CLI, can be called from Python or as a binary
#[tokio::main]
pub async fn cli_main(args: Vec<String>) -> Result<()> {
    let _ = env_logger::try_init_from_env(Env::new().filter(ENV_PROBING_LOGLEVEL));
    let Some(mut cli) = parse_cli(args)? else {
        return Ok(());
    };
    cli.run().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_flag_exits_cleanly_without_error() {
        let result = parse_cli(vec!["probing".into(), "--help".into()]);
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn bare_invocation_shows_help() {
        let result = parse_cli(vec!["probing".into()]);
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn version_flag_exits_cleanly_without_error() {
        let result = parse_cli(vec!["probing".into(), "--version".into()]);
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }
}
