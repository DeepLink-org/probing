# 贡献指南

感谢参与 Probing 开发。本文是 **从 git 仓库开发** 的权威说明。

| 主题 | 文档 |
|------|------|
| **初次参与？从这里开始** | 下文 [欢迎参与开发](#getting-started) |
| 用户安装（PyPI / wheel） | [安装指南](installation.zh.md) |
| 日常开发 bootstrap | [开发环境](#development-setup) |
| Agent 诊断 skill | [Skills 与 Agent](#skills-agents) + [AGENTS.md](https://github.com/DeepLink-org/probing/blob/main/AGENTS.md) |
| PR / 规范 / 行为准则 | [提交变更](#submitting-changes) |
| 文档风格 | [writing.md](writing.md) |

## 欢迎参与开发 {#getting-started}

Probing 是**分层**项目：Rust 里的 SQL 引擎与采集、Python SDK 与 hook、诊断 skill、Web UI。**不必一次学完所有层**——按你的背景选一条贡献路径即可。

### 前 30 分钟

```bash
git clone https://github.com/DeepLink-org/probing.git
cd probing
python3 -m venv .venv && source .venv/bin/activate   # 或：uv venv && source .venv/bin/activate
pip install maturin
make develop
make check-dev
make test-python-regression    # Python 冒烟；完整套件：make test
```

以上通过即表示环境就绪。需要 Rust **stable**（若 `make develop` 因工具链失败，见 [前提条件](#prerequisites)）。可选：`./skills/install.sh`，让 Cursor / Claude / Codex 加载仓库 skill。

本地预览文档：`make docs-install && make docs-serve` → http://127.0.0.1:8000

### 选择贡献方向

| 方向 | 典型工作 | 主要目录 | 先读 | 入门任务示例 |
|------|----------|----------|------|----------------|
| **Skill** | 训练诊断流程 | [`skills/`](https://github.com/DeepLink-org/probing/blob/main/skills/README.md) | [AGENTS.md](https://github.com/DeepLink-org/probing/blob/main/AGENTS.md)、[扩展 — skill](design/extensibility.zh.md#path-2-diagnostic-skill) | 新增 skill、改 `steps.yaml` 里的 SQL、完善 `SKILL.md` |
| **厂商扩展包** | 独立 pip `probing-<vendor>`（entry point 注册 skills + magics） | 模板 `examples/probing-acme/` | [扩展 — 厂商包](design/extensibility.zh.md#path-4-vendor-extension-package-probing-vendor) | 发布 `probing-nvidia` / `probing-huawei` 等 |
| **Python** | 表插件、hook、skill 工具 | `python/probing/`、[`python/probing/skills/`](https://github.com/DeepLink-org/probing/blob/main/python/probing/skills/README.md) | [扩展 — 表插件](design/extensibility.zh.md#path-1-table-plugin-dataclass--table) | `@table` 示例、`tests/regression/skills/` 补测试 |
| **文档与示例** | 概念、教程、排错 | `docs/src/`、`examples/` | [核心概念](guide/concepts.zh.md) | 修正文档、补充 troubleshooting |
| **Rust** | 引擎、服务、采集、CLI | `probing/`（Rust workspace） | [模块化](design/modularity.zh.md) | `probing/core`、`probing/server` 相关 issue |
| **Web UI** | Investigate、各页面 | `web/` | [web/DESIGN.md](https://github.com/DeepLink-org/probing/blob/main/web/DESIGN.md) | Agent 体验（完整 wheel 构建需 `dx`） |

**Skill 数据 vs Python 包：** skill **内容**改仓库根 `skills/`（wheel 复制到 `python/probing/bundled_skills/`）；**加载/安装代码**改 `python/probing/skills/`。**不要**手改 `python/probing/bundled_skills/`（由 `make wheel` 自动生成）。

**两个 `probing/` 目录：** 仓库根 `probing/` 是 **Rust**；`python/probing/` 是 **Python 包**。根目录 `src/lib.rs` 是 PyO3 入口，构建为 `probing._core`。

### 第一个 Pull Request

1. Fork 并建分支：`git checkout -b docs/my-improvement`（或 `feat/…`、`fix/…`）
2. 在**一条**贡献路径上做聚焦改动
3. 跑相关测试：
   - Skill：`python -m probing.skills validate`、`pytest tests/regression/skills/ -q`
   - Python：`pytest tests/unit/probing/…` 或 `tests/regression/…`
   - Rust：`make test-rust-unit` 或 `make test-rust-regression`
   - 仅文档：`make docs`
4. 改了代码则跑 `make lint`
5. 提 PR — 说明**做了什么、为什么**；有关联 issue 请链接

不确定改动该落在哪一层？先开 [Discussion](https://github.com/DeepLink-org/probing/discussions) 或 issue，我们可以帮你指路。

## 前提条件 {#prerequisites}

- **Python** 3.9+
- **Rust（stable 通道）** + **Cargo** — 仓库与 CI 均使用 stable 构建；不需要 nightly
- **maturin** — 构建 `probing._core`（`pip install maturin` 或 `uv pip install maturin`）
- **uv**（可选，推荐）— 常用 `uv venv`；若 venv 无 `pip`，Makefile 会改用 `uv pip`

安装 Rust stable（若尚未安装）：

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup default stable
rustup component add rustfmt clippy
rustc --version   # 应显示 stable，例如 rustc 1.xx.x (…)
```

可选（仅发布 / Web UI）：

- **dioxus-cli**（`dx`）— `make frontend` 构建 Web UI；`make wheel` 将其嵌入 wheel
- **cargo-zigbuild** + **ziglang** — Linux manylinux wheel（`make wheel-ci`）

## 开发环境 {#development-setup}

每个 clone 只需一次：

```bash
git clone https://github.com/DeepLink-org/probing.git
cd probing

# 虚拟环境（二选一）
python3 -m venv .venv && source .venv/bin/activate
# 或：uv venv && source .venv/bin/activate

pip install maturin   # 或：uv pip install maturin
make develop
./skills/install.sh   # 可选：安装到 Cursor / Claude / Codex
```

`make develop` 会依次：

1. **`make core`** — `maturin develop` → `probing._core`、editable `probing.pth`（把 repo `python/` 加入 `sys.path`）、`probing` CLI 进 `PATH`
2. **`install-dev-python-deps`** — 安装 `pytest`、`pyyaml` 等（`pip` / `uv pip` / `ensurepip`）
3. **`python/probing/dev_pth.py install`** — 写入 `probing_hook.pth`，与 wheel 的自动 hook 行为一致
4. **`check-dev`** — 冒烟：`_core`、skill 目录、CLI

随时可检查：

```bash
make check-dev
python python/probing/dev_pth.py status
```

### Site hook（`.pth`）：develop 与 wheel

| 文件 | 来源 | 作用 |
|------|------|------|
| `probing.pth` | maturin | Wheel：`import probing_hook`。Develop：**路径行**指向 repo `python/` |
| `probing_hook.pth` | `make develop`（`dev_pth.py`） | 仅 develop：`import probing_hook`（与路径 `.pth` 配对） |

实现：`python/probing_hook.py` → `python/probing/site_hook.py`。
训练 / 测试：设置 `PROBING=1`（或 `2`、过滤规则 — 见 `site_hook.py`）。测试默认 `PROBING=1`（`tests/conftest.py`）。

在 project venv 内 **`make develop` 后不必再设 `PYTHONPATH=python/`**。

### Makefile 目标（日常）

在仓库根目录执行 **`make help`** 可查看全部目标及一行说明。

| 目标 | 用途 |
|------|------|
| `make develop` / `make dev` | 首次或较大结构变更后 |
| `make core` | 只改了 Rust 扩展 |
| `make check-dev` | 快速检查环境 |
| `make test` | Rust + Python 测试 |
| `make lint` | `ruff check` + `cargo clippy`（workspace + `web/`） |
| `make clippy-fix` | Clippy 自动修复（`--fix --allow-dirty`） |
| `make test-rust` / `make test-python` | 分开跑 |
| `make docs-install` | MkDocs 依赖（首次改文档时） |
| `make docs-serve` | 文档热重载预览 http://127.0.0.1:8000 |
| `make docs` | 静态构建到 `docs/site/` |
| `python -m probing.skills validate` | 校验 `skills/` |
| `make frontend` | 手动构建 `web/dist/`（改 UI 或打 wheel 前） |
| `make wheel` | 发布 wheel（需先有 `web/dist/`；自动 bundle skills + UI） |
| `make install-wheel` | 重装 `dist/probing-*.whl` |

日常：

```bash
source .venv/bin/activate
make test
probing skill list
make core              # 仅 Rust 改动后
```

### 发布 / CI wheel 冒烟

```bash
make frontend && make wheel && make install-wheel
make test-python-wheel
```

### 示例（可选 ML 依赖）

`make develop` **不会**安装 PyTorch。跑 `examples/` 时需自行安装，例如：

```bash
uv pip install torch torchvision
PROBING=1 python examples/tracing.py
```

见 [examples/README.md](https://github.com/DeepLink-org/probing/blob/main/examples/README.md)。

## Skills 与 Agent {#skills-agents}

- **编写**：仓库根 `skills/`（`SKILL.md`、`steps.yaml`、`catalog.yaml`）
- **安装到 IDE**：`./skills/install.sh` 或 `probing skill install`
- **打进 wheel**：`make wheel` 自动复制到 `python/probing/bundled_skills/`、`python/probing/bundled_web/`
- **说明**：`skills/README.md`、[扩展机制 — 诊断 skill](design/extensibility.zh.md#path-2-diagnostic-skill)

## 开发流程

### 运行测试

测试分为 **单元** 与 **回归** 两层，目录约定见 [`tests/README.md`](https://github.com/DeepLink-org/probing/blob/main/tests/README.md)。

| 类型 | Rust | Python |
|------|------|--------|
| 单元 | 源码内 `#[cfg(test)]` | `tests/unit/probing/` 镜像 `python/probing/` |
| 回归 | `tests/regression/rust/probing/**` + `probing/macros/tests/` | `tests/regression/`（含 `spec/api_spec.json`） |

```bash
make test
make test-rust-unit
make test-rust-regression
make test-python-unit
make test-python-regression
make test-python
make coverage
```

**Rust 例外：** `probing/macros/tests/` 为 proc-macro 外部 crate 测试，必须保留独立文件。

### 代码风格

**Python：** `ruff`（lint + format）、`mypy`

```bash
make fmt              # Python: ruff format + fix；Rust: rustfmt
make lint-python      # ruff check + ruff format --check
mypy python/probing
```

**Rust：** `rustfmt`、`clippy`（共享规则在 `clippy.toml`；严格 lint **按 crate 渐进启用**，见各 crate 的 `Cargo.toml` `[lints]`）

```bash
cargo fmt --all
make lint-core          # probing-core（已启用 clippy::all）
make lint-rust          # workspace + web/，warning 视为 error
make clippy-fix         # 自动修复（提交前请 review diff）
```

CI 会跑 Clippy。改 `probing-core` 后推送前建议 `make lint-core`；全 workspace 仍会因未 lint 的 crate 失败，其余 crate 会逐个跟进。

当前进度：`probing-core` 已启用 `clippy::all`（`pedantic`/`nursery` 关闭，协议相关 allow 见 `probing/core/Cargo.toml`）。下一批候选：`probing-proto`、`probing-memtable`。

### 构建文档

在**仓库根目录**（与 `make test`、`make develop` 同一入口）：

```bash
make docs-install   # 首次：MkDocs + i18n + mkdocstrings
make docs-serve     # http://127.0.0.1:8000，保存后自动刷新
make docs           # 静态构建 → docs/site/
```

进阶：`cd docs && make deploy` 发布到 GitHub Pages。

## 项目结构

```
probing/                          # 仓库根
├── skills/                       # skill 数据（编写）— 见 skills/README.md
├── python/
│   ├── probing/                  # Python 包（不是 Rust）
│   │   ├── skills/               # skill 加载/安装代码 — 见 python/probing/skills/README.md
│   │   ├── web_assets.py         # wheel _web/ + editable web/dist → PROBING_ASSETS_ROOT
│   │   ├── bundled_skills/       # wheel 打包的 skill 数据（编写在 repo-root skills/）
│   │   └── bundled_web/          # wheel 打包的 UI（make frontend）
│   ├── probing_hook.py
│   └── probing.pth
├── src/lib.rs                    # PyO3 → probing._core
├── probing/                      # Rust workspace
├── web/                          # Dioxus UI（`make frontend` → web/dist/）
├── tests/                        # 见 tests/README.md
├── examples/
└── docs/src/
```

分层与依赖规则：[模块化](design/modularity.zh.md)。
Agent 工作流：[AGENTS.md](https://github.com/DeepLink-org/probing/blob/main/AGENTS.md)。

## 提交变更 {#submitting-changes}

### Pull Request 流程

1. Fork 并建分支：`git checkout -b feature/xxx`
2. 修改 + 测试 + 文档
3. `make test && make lint`
4. 提交 PR，说明清晰、范围聚焦

### 提交消息

遵循 [Conventional Commits](https://www.conventionalcommits.org/)：`feat:`、`fix:`、`docs:`、`test:`、`chore:` 等。

### 代码审查

- PR 聚焦单一主题；行为变更需测试
- 安装 / 开发流程变更需同步文档

## 贡献方向

各层都欢迎 PR——**不要求**一开始就懂 Rust 或前端。

| 标签 / 领域 | 示例 |
|-------------|------|
| **Skill 与文档** | 新诊断、文档改进、翻译 |
| **`good-first-issue`** | GitHub 上的入门 issue |
| **Python 插件** | `@table` 采集、skill 工具链 |
| **Rust / Web** | 引擎、服务、UI — 适合有对应背景的同学 |
| **测试** | 单元/回归覆盖 — 见 [`tests/README.md`](https://github.com/DeepLink-org/probing/blob/main/tests/README.md) |

大功能请先开 issue 讨论，再做大 PR。

## 获取帮助

- **GitHub Issues** — Bug 与功能
- **Discussions** — 问题与设计讨论

## 行为准则

请保持尊重与建设性。贡献采用 Apache 2.0 许可证。
