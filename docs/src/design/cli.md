# CLI command tree

**Status:** draft · **SSOT** for grouping and migration · Code: `probing/cli/src/cli/{commands,help,mod}.rs`

**Legend:** `T` = `-t/--target` · `*` = needs T · `—` = no T · `L` = Linux only · `H` = hidden

---

## Principle: flat invocation, grouped help

| Dimension | Rule |
|-----------|------|
| **Invocation** | Single-level: `probing [-v] [-t T] <cmd> [args…]` |
| **Help** | `probing --help` grouped under Processes / Analyze / Diagnose / Runtime / Agent |
| **Consolidation** | Merge `cluster query` → `query --global`; `cluster nodes` → top-level `nodes` (TBD) |
| **Exceptions** | `skill` keeps subcommands; `bench` / `store` hidden |

### Help implementation (option B, adopted)

clap 4.5–4.6 does **not** support multiple subcommand headings ([clap#1553](https://github.com/clap-rs/clap/issues/1553) still open). We use a **custom root help template**:

1. Root `help_template` **omits `{subcommands}`**; keeps `{about}` / `{usage}` / `{options}`.
2. `{after-help}` injects grouped command tables from `help.rs`.
3. Section titles and blurbs in **`help.rs` → `SECTIONS`**; per-command one-liners from clap `about` on each subcommand (`commands.rs` remains per-command wording SSOT).
4. `probing <cmd> --help` still uses default clap per-command help.

```text
Cli::build_command()
  → CommandFactory::command()
  → help::apply_grouped_root_help()
```

---

## Help sections (rationale)

| Section | Commands | Notes |
|---------|----------|-------|
| **Processes** | `inject`, `launch`, `list` | Establish or discover probing on a process; avoid “Attach” (ptrace jargon) |
| **Analyze** | `query`, `tables`, `cluster` | SQL and catalog; `cluster` until merged into `query --global` / `nodes` |
| **Diagnose** | `eval`, `repl`, `backtrace` | Interactive, immediate inspection |
| **Runtime** | `memory`, `config`, `flamegraph`, `rdma` | Runtime state and profiling |
| **Agent** | `skill`, `mcp` | Coding-agent integration: skills and MCP config |

---

## Target invocation tree

```text
probing [-v] [-t T] <cmd> …

inject(L*)*  launch(L)—  list—
query*  tables*  nodes*          # TBD: merge cluster into query/nodes
eval*  repl*  backtrace*  flamegraph*  rdma*
memory*  config*
skill  list— | install— | update— | run* …
mcp  url* | config*
bench(H)—  store(H)—
```

---

## Maintenance

When adding or moving a top-level subcommand:

1. Register in `commands.rs` (with `about`)
2. Add name to `help.rs` → `SECTIONS`
3. Update this doc

---

## Related

[federation.md](federation.md) · [skills](../guide/skills.md) · [api-reference](../api-reference.md)
