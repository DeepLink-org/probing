# CLI 命令树

**状态：** 草案 · **SSOT** 命令分组与迁移 · 实现：`probing/cli/src/cli/{commands,help,mod}.rs`

**约定：** `T` = `-t/--target` · `*` = 需要 T · `—` = 不需要 T · `L` = 仅 Linux · `H` = hidden

---

## 设计原则：平铺调用，分组帮助

| 维度 | 规则 |
|------|------|
| **调用** | 单层子命令：`probing [-v] [-t T] <cmd> [args…]` |
| **帮助** | `probing --help` 按 Processes / Analyze / Diagnose / Runtime / Agent **分组展示** |
| **收敛** | 合并 `cluster query` → `query --global`；`cluster nodes` → 顶层 `nodes`（待做） |
| **例外** | `skill` 保留二级子命令；`bench`/`store` 隐藏 |

### Help 实现（方案 B，已采用）

clap 4.5–4.6 **不支持**同级子命令多 heading（[clap#1553](https://github.com/clap-rs/clap/issues/1553) 未 merge）。采用 **自定义根 help 模板**：

1. 根 `help_template` **去掉 `{subcommands}`**，只保留 `{about}` / `{usage}` / `{options}`。
2. `{after-help}` 注入 `help.rs` 渲染的分组命令表。
3. 分组标题与组说明在 **`help.rs` → `SECTIONS`**；各命令单行说明来自 subcommand 的 clap `about`（`commands.rs` 仍为 per-command 文案 SSOT）。
4. `probing <cmd> --help` 仍走 clap 默认 per-command help。

```text
Cli::build_command()
  → CommandFactory::command()
  → help::apply_grouped_root_help()   # template + after_long_help
```

---

## Help 分组（命名 rationale）

| 组 | 命令 | 说明 |
|----|------|------|
| **Processes** | `inject`, `launch`, `list` | 与目标进程建立/发现 probing 关系；不用「Attach」（用户不熟悉 ptrace 术语） |
| **Analyze** | `query`, `tables`, `cluster` | SQL 与表目录；cluster 暂保留至 `query --global` / `nodes` 落地 |
| **Diagnose** | `eval`, `repl`, `backtrace` | 交互式、即时检查 |
| **Runtime** | `memory`, `config`, `flamegraph`, `rdma` | 运行时状态与 profiling（资源、配置、采样、I/O） |
| **Agent** | `skill`, `mcp` | 与 coding agent 集成：诊断 skill 与 MCP 端点配置 |

---

## 当前（v0.2.5）

```text
probing [-v] [-t T] <cmd> …

inject(L*)*  launch(L)—  list—  config*  tables*  memory*
query|q*  cluster/query*  cluster/nodes*
eval*  repl*  backtrace*  flamegraph*  rdma*
skill/{list—,install—,update—,run*}  mcp/{url*,config*}
bench(H)—  store(H)—
external→probing-<cmd>
```

---

## 目标 — 平铺命令 + 分组 `--help`

### 调用树

```text
probing [-v] [-t T] <cmd> …

inject(L*)*     [-D define…]
launch(L)—      [-r] <cmd…>
list—           [--tree] [--verbose]

query*          <sql> [-f fmt] [--global|--local|--flat]     # 待做：吸收 cluster query
tables*         [--all] [-f fmt]
nodes*          # 待做：吸收 cluster nodes

memory*  config*  flamegraph*  rdma*
skill  list— | install— | update— | run* …
mcp  url* | config*
bench(H)—  store(H)—
```

### 帮助树（`probing --help` 输出）

```text
Processes — Start probing on a process, wrap a new command, or list probed PIDs
  inject        Inject libprobing into a running process (Linux ptrace)
  launch        Launch a command with probing enabled (Linux)
  list          List processes that already have probing enabled

Analyze — Run SQL, inspect table catalog, fan out across cluster nodes
  query         Query data from the target process
  tables        List queryable tables in the target process
  cluster       On-demand cluster SQL fan-out and node listing

Diagnose — Interactive inspection — Python eval, REPL, stack traces
  eval          Evaluate Python code in the target process
  repl          Interactive Python REPL session
  backtrace     Show the backtrace of the target process or thread

Runtime — Runtime state and profiling — memory, config, flamegraphs, RDMA flows
  memory        Show memory usage (host RSS and GPU memory) of the target process
  config        Display or modify the configuration
  flamegraph    Fetch a flamegraph (CPU/pprof or PyTorch) from the target process
  rdma          Get RDMA flow of the target process or thread

Agent — Integrate coding agents — diagnostic skills and MCP server config
  skill         Run structured diagnostic skills (shared with Web Agent)
  mcp           MCP endpoint URL and agent config for the target probing server

Run `probing <cmd> --help` for command-specific options.
```

---

## 典型用法

```text
probing inject -t PID
probing query -t PID 'SELECT …'
probing skill run -t PID health_overview
probing mcp config -t host:8080
probing backtrace -t PID
```

---

## 变更摘要（相对 v0.2.5）

| 状态 | 项 |
|------|-----|
| ✅ | 分组根 `--help`（`help.rs`） |
| ✅ | Help 组名：Processes / Analyze / Diagnose / Runtime / Agent |
| ✅ | `probing mcp url|config` — MCP 端点与 agent 配置片段 |
| 待做 | `cluster query` → `query --global` |
| 待做 | `cluster nodes` → 顶层 `nodes` |
| ✅ | 取消隐式 inject（无子命令不再 inject） |
| ✅ | CLI 导入跳过 `#[ctor]` 引擎启动（`__init__.py` CLI 轻量路径） |

---

## 待定

| # | 问题 | 默认 |
|---|------|------|
| 1 | positional `[T]` 作 `-t` 糖 | TBD |
| 2 | external 插件 | TBD |

---

## 维护

新增/移动顶层子命令时：

1. 在 `commands.rs` 注册 subcommand（含 `about`）
2. 在 `help.rs` → `SECTIONS` 里加入组与命令名
3. 更新本文调用树

---

## 相关

[federation.zh.md](federation.zh.md) · [skills](../guide/skills.md) · [api-reference](../api-reference.md)
