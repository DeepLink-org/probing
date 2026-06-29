//! Grouped root `--help` for flat subcommands (clap has no multi-heading support yet).
//!
//! Section titles and blurbs live here; per-command one-liners come from clap `about`
//! on each subcommand in `commands.rs`.

use clap::Command;

const ROOT_HELP_TEMPLATE: &str = "\
{before-help}{about-with-newline}\
{usage-heading} {usage}\n\
\n\
{options}\n\
{after-help}\
";

struct HelpSection {
    /// Section title shown in `probing --help`.
    heading: &'static str,
    /// One-line intent (shown under the title).
    blurb: &'static str,
    commands: &'static [&'static str],
}

/// Help grouping — not argv paths. See `docs/src/design/cli.md` § Help sections.
#[cfg(target_os = "linux")]
const SECTIONS: &[HelpSection] = &[
    HelpSection {
        heading: "Processes",
        blurb: "Start probing on a process, wrap a new command, or list probed PIDs",
        commands: &["inject", "launch", "list"],
    },
    HelpSection {
        heading: "Analyze",
        blurb: "Run SQL, inspect table catalog, fan out across cluster nodes",
        commands: &["query", "tables", "cluster"],
    },
    HelpSection {
        heading: "Diagnose",
        blurb: "Interactive inspection — Python eval, REPL, stack traces",
        commands: &["eval", "repl", "backtrace"],
    },
    HelpSection {
        heading: "Runtime",
        blurb: "Runtime state and profiling — memory, config, flamegraphs, RDMA flows",
        commands: &["memory", "config", "flamegraph", "rdma"],
    },
    HelpSection {
        heading: "Agent",
        blurb: "Diagnostic skills and MCP — start with `skill run health_overview`",
        commands: &["skill", "mcp"],
    },
];

#[cfg(not(target_os = "linux"))]
const SECTIONS: &[HelpSection] = &[
    HelpSection {
        heading: "Processes",
        blurb: "List processes that already have probing enabled",
        commands: &["list"],
    },
    HelpSection {
        heading: "Analyze",
        blurb: "Run SQL, inspect table catalog, fan out across cluster nodes",
        commands: &["query", "tables", "cluster"],
    },
    HelpSection {
        heading: "Diagnose",
        blurb: "Interactive inspection — Python eval, REPL, stack traces",
        commands: &["eval", "repl", "backtrace"],
    },
    HelpSection {
        heading: "Runtime",
        blurb: "Runtime state and profiling — memory, config, flamegraphs, RDMA flows",
        commands: &["memory", "config", "flamegraph", "rdma"],
    },
    HelpSection {
        heading: "Agent",
        blurb: "Diagnostic skills and MCP — start with `skill run health_overview`",
        commands: &["skill", "mcp"],
    },
];

/// Replace the default flat subcommand list with grouped sections in `{after-help}`.
pub fn apply_grouped_root_help(cmd: &mut Command) {
    let grouped = render_grouped_subcommands(cmd);
    *cmd = cmd
        .clone()
        .help_template(ROOT_HELP_TEMPLATE)
        .after_help(grouped.clone())
        .after_long_help(grouped);
}

fn render_grouped_subcommands(root: &Command) -> String {
    let mut out = String::new();
    let name_width = 14_usize;

    for section in SECTIONS {
        let mut lines = Vec::new();
        for name in section.commands {
            let Some(sub) = root.find_subcommand(name) else {
                continue;
            };
            if sub.is_hide_set() {
                continue;
            }
            let about = sub
                .get_about()
                .or_else(|| sub.get_long_about())
                .map(|s| s.to_string())
                .unwrap_or_default();
            lines.push((name, about));
        }
        if lines.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        use std::fmt::Write as _;
        let _ = writeln!(out, "{} — {}", section.heading, section.blurb);
        for (name, about) in lines {
            let _ = writeln!(out, "  {name:<name_width$}{about}");
        }
    }

    out.push_str(
        "\nMost commands need `-t PID` or `-t host:port` \
         (exceptions: list, skill list/install/update).\n\
         Run `probing <cmd> --help` for command-specific options.\n",
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    use crate::cli::Cli;

    #[test]
    fn grouped_help_lists_all_sections() {
        let cmd = Cli::command();
        let text = render_grouped_subcommands(&cmd);
        for heading in [
            "Processes —",
            "Analyze —",
            "Diagnose —",
            "Runtime —",
            "Agent —",
        ] {
            assert!(
                text.contains(heading),
                "missing section {heading} in:\n{text}"
            );
        }
        assert!(text.contains("query"));
        assert!(text.contains("skill"));
        assert!(text.contains("mcp"));
        assert!(!text.contains("Profile —"));
        assert!(!text.contains("Skills —"));
        assert!(!text.contains("Attach"));
        assert!(text.contains("skill run health_overview"));
        assert!(text.contains("`-t PID`"));
    }

    #[test]
    fn root_short_help_includes_grouped_commands() {
        let mut cmd = crate::cli::Cli::build_command();
        let mut buf = Vec::new();
        cmd.write_help(&mut buf).unwrap();
        let text = String::from_utf8(buf).expect("utf8 help");
        assert!(text.contains("Processes —"));
        assert!(text.contains("Runtime —"));
        assert!(text.contains("Agent —"));
    }
}
