//! MCP (Model Context Protocol) helpers for coding-agent integration.

use anyhow::{Context, Result};
use clap::Subcommand;

use crate::cli::ctrl::ProbeEndpoint;

#[derive(Subcommand, Debug, Clone)]
pub enum McpCommand {
    /// Print the MCP endpoint URL for the target probing server
    Url,
    /// Print a ready-to-paste MCP server config snippet (Cursor / Claude Code)
    Config {
        /// Server name in the agent config JSON
        #[arg(long, default_value = "probing")]
        name: String,
    },
}

pub async fn run(ctrl: ProbeEndpoint, cmd: McpCommand) -> Result<()> {
    let base = mcp_base_url(&ctrl).await?;
    let url = format!("{base}/mcp");
    match cmd {
        McpCommand::Url => {
            println!("{url}");
        }
        McpCommand::Config { name } => {
            let snippet = serde_json::json!({
                "mcpServers": {
                    name: { "url": url }
                }
            });
            println!("{}", serde_json::to_string_pretty(&snippet)?);
        }
    }
    Ok(())
}

async fn mcp_base_url(ctrl: &ProbeEndpoint) -> Result<String> {
    match ctrl {
        ProbeEndpoint::Remote { addr } => Ok(format!("http://{addr}")),
        ProbeEndpoint::Local { .. } | ProbeEndpoint::Ptrace { .. } => {
            let addr = ctrl.get("/config/server.address").await.context(
                "could not read server.address from target; \
                     set PROBING_PORT (or server.address) on the training process",
            )?;
            Ok(normalize_listen_addr(addr.trim()))
        }
        ProbeEndpoint::Launch { .. } => {
            anyhow::bail!(
                "MCP is served by the probing HTTP server; use `-t <pid>` or `-t host:port`"
            )
        }
    }
}

fn normalize_listen_addr(addr: &str) -> String {
    let host_port = if let Some(port) = addr.strip_prefix("0.0.0.0:") {
        format!("127.0.0.1:{port}")
    } else if let Some(port) = addr.strip_prefix("[::]:") {
        format!("127.0.0.1:{port}")
    } else if let Some(port) = addr.strip_prefix(":::") {
        format!("127.0.0.1:{port}")
    } else {
        addr.to_string()
    };

    if host_port.starts_with("http://") || host_port.starts_with("https://") {
        host_port
    } else {
        format!("http://{host_port}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_wildcard_bind() {
        assert_eq!(
            normalize_listen_addr("0.0.0.0:8080"),
            "http://127.0.0.1:8080"
        );
        assert_eq!(
            normalize_listen_addr("127.0.0.1:18080"),
            "http://127.0.0.1:18080"
        );
    }
}
