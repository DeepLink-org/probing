# 版本兼容性

本页记录 Probing 的版本兼容性和更新日志。

## 当前版本

**Probing v0.6.x** (最新)

## 系统要求

### Python 版本

| Probing 版本 | Python 支持 |
|--------------|-------------|
| 0.6.x | Python 3.9 - 3.12 |
| 0.5.x | Python 3.8 - 3.11 |

### PyTorch 版本

| Probing 版本 | PyTorch 支持 |
|--------------|--------------|
| 0.6.x | PyTorch 2.0+ |
| 0.5.x | PyTorch 1.13+ |

### 操作系统

- **Linux**: 完全支持（生产环境推荐）
- **macOS**: 完全支持（Intel 和 Apple Silicon）
- **Windows**: 实验性支持（推荐使用 WSL2）

## 更新日志

### v0.6.0

**新功能**

- 基于 DataFusion 的 SQL 查询引擎
- 文档支持 Mermaid 图表
- 改进的分布式调试支持
- 新增 `torch_trace` 表用于 PyTorch 性能分析

**破坏性变更**

- 废弃 `probing.trace()` API，改用 `probing.enable_torch_profiling()`
- 配置格式从 JSON 改为 TOML

**Bug 修复**

- 修复长时间运行会话中的内存泄漏
- 改进无效 SQL 查询的错误消息

### v0.5.0

**新功能**

- 初始 PyTorch 性能分析支持
- 内存分析能力
- 基础 SQL 查询支持

**Bug 修复**

- 各种稳定性改进

## 升级指南

### 从 v0.5.x 升级到 v0.6.x

1. 如需要，升级 Python 到 3.9+
2. 如需要，升级 PyTorch 到 2.0+
3. 更新 Probing：

```bash
pip install --upgrade probing
```

4. 更新配置文件（如果使用自定义配置）：

```python
# 旧格式 (v0.5.x)
probing.trace(enabled=True)

# 新格式 (v0.6.x)
probing.enable_torch_profiling()
```

## 废弃策略

- 主版本变更可能包含破坏性变更
- 次版本变更保持向后兼容性
- 废弃功能在移除前至少显示一个次版本的警告

## 功能支持矩阵

| 功能 | v0.5.x | v0.6.x |
|------|--------|--------|
| 基础性能分析 | ✅ | ✅ |
| SQL 查询 | 部分 | ✅ |
| PyTorch 跟踪 | 基础 | 完整 |
| 内存分析 | 基础 | 完整 |
| 分布式支持 | ❌ | ✅ |
| 自定义表 | ❌ | ✅ |
| Web UI | ❌ | Beta |

## 报告问题

如需报告 Bug 或功能请求，请使用 [GitHub Issue Tracker](https://github.com/DeepLink-org/probing/issues)。

报告问题时，请包含：

- Probing 版本 (`pip show probing`)
- Python 版本 (`python --version`)
- PyTorch 版本（如适用）
- 操作系统
- 最小复现示例
