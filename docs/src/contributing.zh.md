# 贡献指南

感谢您对贡献 Probing 的兴趣！本指南将帮助您开始。

## 入门

### 前提条件

- Python 3.9+
- Rust（最新稳定版）
- maturin（用于构建 Python 扩展）

### 开发环境设置

1. 克隆仓库：

```bash
git clone https://github.com/DeepLink-org/probing.git
cd probing
```

2. 创建虚拟环境：

```bash
python -m venv .venv
source .venv/bin/activate  # Linux/macOS
# 或
.venv\Scripts\activate  # Windows
```

3. 安装开发依赖：

```bash
pip install -e ".[dev]"
```

4. 构建 Rust 扩展：

```bash
maturin develop
```

## 开发流程

### 运行测试

```bash
# 运行所有测试
make test

# 运行特定测试
pytest tests/test_specific.py -v

# 运行带覆盖率的测试
make coverage
```

### 代码风格

我们使用以下工具保证代码质量：

**Python：**

- `ruff` 用于代码检查和格式化
- `mypy` 用于类型检查

```bash
# 格式化代码
ruff format .

# 检查代码规范
ruff check .

# 类型检查
mypy python/probing
```

**Rust：**

- `rustfmt` 用于格式化
- `clippy` 用于代码检查

```bash
# 格式化
cargo fmt

# 检查
cargo clippy --all --tests --benches -- -D warnings
```

### 构建文档

```bash
cd docs
make install  # 安装依赖
make serve    # 在 http://127.0.0.1:8000 预览
```

## 提交变更

### Pull Request 流程

1. Fork 仓库
2. 创建功能分支：

```bash
git checkout -b feature/your-feature-name
```

3. 进行修改
4. 运行测试和检查：

```bash
make test
make lint
```

5. 使用描述性消息提交：

```bash
git commit -m "feat: 添加新功能描述"
```

6. 推送并创建 Pull Request

### 提交消息格式

我们遵循 [Conventional Commits](https://www.conventionalcommits.org/)：

- `feat:` - 新功能
- `fix:` - Bug 修复
- `docs:` - 文档变更
- `style:` - 代码风格变更（格式化）
- `refactor:` - 代码重构
- `test:` - 测试变更
- `chore:` - 构建/工具变更

### 代码审查

所有提交都需要代码审查。请：

- 保持 PR 专注于单一变更
- 为新功能添加测试
- 根据需要更新文档
- 及时响应审查反馈

## 项目结构

```
probing/
├── python/             # Python 源代码
│   └── probing/        # 主 Python 包
├── probing/            # Rust crates
│   ├── core/           # 核心功能
│   ├── server/         # HTTP 服务器
│   ├── extensions/     # Python/PyTorch 扩展
│   └── cli/            # 命令行界面
├── tests/              # Python 测试
├── docs/               # 文档
└── examples/           # 使用示例
```

## 贡献领域

### 适合新手的 Issues

在 GitHub 上查找标记为 `good-first-issue` 的问题。这些适合新贡献者。

### 文档

- 改进现有文档
- 添加更多示例
- 翻译文档

### 测试

- 增加测试覆盖率
- 编写集成测试
- 性能基准测试

### 功能

- 查看路线图和开放的 issues
- 大型变更前请先讨论

## 获取帮助

- **GitHub Issues**：用于 Bug 和功能请求
- **Discussions**：用于问题和想法讨论

## 行为准则

请在所有互动中保持尊重和建设性。我们致力于为每个人提供友好的环境。

## 许可证

通过贡献，您同意您的贡献将根据项目的 Apache 2.0 许可证进行许可。
