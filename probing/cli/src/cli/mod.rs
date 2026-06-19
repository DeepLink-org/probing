use anyhow::Result;
use clap::Parser;
use probing_proto::prelude::Query;

pub mod bench;
pub mod cluster;
pub mod commands;
pub mod ctrl;
pub mod doctor;
pub mod repl;

pub mod store;

#[cfg(target_os = "linux")]
pub mod inject;

#[cfg(target_os = "linux")]
pub mod process_monitor;

#[cfg(target_os = "linux")]
use process_monitor::ProcessMonitor;

mod ptree;

use crate::cli::ctrl::ProbeEndpoint;
use crate::table::OutputFormat;
use commands::{Commands, FlamegraphKind};
use once_cell::sync::Lazy;

fn get_build_info() -> String {
    let mut info = env!("CARGO_PKG_VERSION").to_string();

    if let Some(timestamp) = option_env!("VERGEN_BUILD_TIMESTAMP") {
        info.push_str(&format!("\nBuild Timestamp: {timestamp}"));
    }

    if let Some(rustc_version) = option_env!("VERGEN_RUSTC_SEMVER") {
        info.push_str(&format!("\nrustc version: {rustc_version}"));
    }

    info
}

static BUILD_INFO: Lazy<String> = Lazy::new(get_build_info);

/// Probing CLI - A performance and stability diagnostic tool for AI applications
#[derive(Parser, Debug)]
#[command(version = BUILD_INFO.as_str())]
pub struct Cli {
    /// Enable verbose mode
    #[arg(short, long, global = true)]
    verbose: bool,

    /// target process, PID (e.g., 1234) for local process, and <ip>:<port> for remote process
    #[arg(short, long)]
    target: Option<String>,

    #[command(subcommand)]
    command: Option<Commands>,
}

impl Cli {
    pub async fn run(&mut self) -> Result<()> {
        // Handle external commands first to avoid target requirement
        if let Some(Commands::External(args)) = &self.command {
            std::env::set_var("PROBING_ENDPOINT", self.target.clone().unwrap_or_default());
            return handle_external_command(args);
        }

        // Handle commands that don't need a target
        match &self.command {
            Some(Commands::List { verbose, tree }) => {
                return self.handle_list_command(*verbose, *tree).await;
            }
            #[cfg(target_os = "linux")]
            Some(Commands::Launch { recursive, args }) => {
                return ProcessMonitor::new(args, *recursive)?.monitor().await;
            }
            Some(Commands::Store(cmd)) => {
                return cmd.run().await;
            }
            Some(Commands::Bench(cmd)) => {
                return cmd.run();
            }
            Some(Commands::Doctor(doctor::DoctorCommand::List)) => {
                return doctor::list_playbooks_sync();
            }
            _ => {}
        }

        // For other commands, we need a target
        let target = self.target.clone().unwrap_or("0".to_string());
        let ctrl: ProbeEndpoint = target.as_str().try_into()?;
        self.execute_command(ctrl).await
    }

    async fn handle_list_command(&self, verbose: bool, tree: bool) -> Result<()> {
        match ptree::collect_probe_processes().await {
            Ok(processes) => {
                if processes.is_empty() {
                    println!("No processes with injected probes found.");
                    return Ok(());
                }

                if tree {
                    let tree_nodes = ptree::build_process_tree(processes);
                    println!("Processes with injected probes (tree view):");
                    ptree::print_process_tree(&tree_nodes, verbose, "");
                } else {
                    println!("Processes with injected probes:");
                    for p in processes {
                        println!("{}", ptree::format_process(&p, verbose));
                    }
                }
            }
            Err(e) => {
                eprintln!("Error listing processes: {e}");
            }
        }
        Ok(())
    }

    async fn handle_tables_command(
        &self,
        ctrl: ProbeEndpoint,
        all: bool,
        format: OutputFormat,
    ) -> Result<()> {
        let expr = if all {
            "select table_catalog, table_schema, table_name, table_type \
             from information_schema.tables order by table_schema, table_name"
                .to_string()
        } else {
            "select table_schema, table_name, table_type \
             from information_schema.tables \
             where table_schema not in ('information_schema') \
             order by table_schema, table_name"
                .to_string()
        };
        ctrl::query_with_format(ctrl, Query::new(expr), format).await
    }

    async fn handle_memory_command(
        &self,
        ctrl: ProbeEndpoint,
        limit: usize,
        format: OutputFormat,
    ) -> Result<()> {
        let cpu_expr = format!(
            "select ts, comm, rss_kb, thread_count from cpu.utilization \
             where scope = 'process' order by ts desc limit {limit}"
        );
        let mut printed = false;
        match ctrl.query(Query::new(cpu_expr)).await {
            Ok(df) if df.cols.iter().any(|c| !c.is_empty()) => {
                println!("Host memory (cpu.utilization):");
                crate::table::render(&df, format);
                printed = true;
            }
            Ok(_) => {}
            Err(e) => eprintln!("host memory unavailable: {e}"),
        }

        let gpu_expr = format!(
            "select ts, device_id, name, used_bytes, total_bytes, mem_used_pct \
             from gpu.utilization order by ts desc limit {limit}"
        );
        match ctrl.query(Query::new(gpu_expr)).await {
            Ok(df) if df.cols.iter().any(|c| !c.is_empty()) => {
                if printed {
                    println!();
                }
                println!("GPU memory (gpu.utilization):");
                crate::table::render(&df, format);
                printed = true;
            }
            Ok(_) => {}
            Err(e) => log::debug!("gpu memory unavailable: {e}"),
        }

        if !printed {
            println!(
                "No memory samples available. Ensure CPU/GPU sampling is enabled in the target process."
            );
        }
        Ok(())
    }

    async fn handle_flamegraph_command(
        &self,
        ctrl: ProbeEndpoint,
        kind: FlamegraphKind,
        output: Option<String>,
        json: bool,
    ) -> Result<()> {
        let bytes = ctrl.flamegraph(kind.as_str(), json).await?;
        match output {
            Some(path) => {
                std::fs::write(&path, &bytes)?;
                eprintln!("flamegraph ({}) written to {path}", kind.as_str());
            }
            None => {
                use std::io::Write;
                std::io::stdout().write_all(&bytes)?;
                std::io::stdout().flush()?;
            }
        }
        Ok(())
    }

    async fn execute_command(&self, ctrl: ProbeEndpoint) -> Result<()> {
        if self.command.is_none() {
            #[cfg(target_os = "linux")]
            inject::InjectCommand::default().run(ctrl.clone()).await?;

            return Ok(());
        }
        let command = self.command.as_ref().unwrap();
        match command {
            #[cfg(target_os = "linux")]
            Commands::Inject(cmd) => cmd.run(ctrl).await,
            Commands::Config { options, setting } => {
                let options_cfg = options.to_cfg();

                let query_expr = match (setting, options_cfg) {
                    (Some(setting_str), Some(opts_str)) => {
                        let setting = if !setting_str.starts_with("set ")
                            && !setting_str.starts_with("SET ")
                        {
                            format!("set {setting_str}")
                        } else {
                            setting_str.clone()
                        };
                        format!("{setting}; {opts_str}")
                    }
                    (Some(setting_str), None) => {
                        if !setting_str.starts_with("set ") && !setting_str.starts_with("SET ") {
                            format!("set {setting_str}")
                        } else {
                            setting_str.clone()
                        }
                    }
                    (None, Some(opts_str)) => opts_str,
                    (None, None) => {
                        "select * from information_schema.df_settings where name like 'probing.%';"
                            .to_string()
                    }
                };

                ctrl::query(
                    ctrl,
                    Query {
                        expr: query_expr,
                        opts: None,
                    },
                )
                .await
            }
            Commands::Backtrace { tid } => ctrl.backtrace(*tid).await,
            Commands::Rdma { hca_name } => {
                let hca_name = hca_name.clone().unwrap_or_default();
                ctrl.rdma(hca_name).await
            }
            Commands::Eval { code } => ctrl.eval(code.clone()).await,
            Commands::Query { query, format } => {
                ctrl::query_with_format(ctrl, Query::new(query.clone()), *format).await
            }
            Commands::Tables { all, format } => {
                self.handle_tables_command(ctrl, *all, *format).await
            }
            Commands::Memory { limit, format } => {
                self.handle_memory_command(ctrl, *limit, *format).await
            }
            Commands::Flamegraph { kind, output, json } => {
                self.handle_flamegraph_command(ctrl, *kind, output.clone(), *json)
                    .await
            }
            Commands::Cluster(cmd) => cluster::run(ctrl, cmd.clone()).await,
            Commands::Doctor(cmd) => doctor::run(ctrl, cmd.clone()).await,
            Commands::Repl => repl::start_repl(ctrl).await,
            // These commands are handled in run() method and don't need a target
            Commands::Launch { .. }
            | Commands::List { .. }
            | Commands::Store(..)
            | Commands::Bench(..)
            | Commands::External(..) => {
                unreachable!("These commands should be handled in run() method")
            }
        }
    }
}

fn handle_external_command(args: &[String]) -> Result<()> {
    if args.is_empty() {
        eprintln!("Command not specified. Please provide a subcommand.");
        std::process::exit(1);
    }

    let subcommand = &args[0];
    let external_bin = format!("probing-{subcommand}");

    let status = std::process::Command::new(&external_bin)
        .args(&args[1..])
        .status();

    match status {
        Ok(exit_status) => std::process::exit(exit_status.code().unwrap_or(1)),
        Err(e) => {
            eprintln!("Error finding external command '{external_bin}'\n\t{e}");
            std::process::exit(1);
        }
    }
}
