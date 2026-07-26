# Getting started

入门与冒烟：tracing、hooks、ExternalTable、父子进程注入。

## 依赖

- `torch`（`tracing.py` / `hooks.py`）
- 其余脚本仅需 probing

## 运行

```bash
./examples/getting-started/run_tracing.sh          # 推荐：span / phase
./examples/getting-started/run_hooks.sh
./examples/getting-started/run_external_table.sh
./examples/getting-started/run_test.sh --depth 2
```

或：

```bash
PROBING=1 python examples/getting-started/tracing.py
```

## 文件

| 文件 | 说明 |
|------|------|
| `tracing.py` | Tracing 入门（~80 行） |
| `hooks.py` | Torch module hook → ExternalTable |
| `external_table.py` | ExternalTable API |
| `test_probing.py` | 父子进程 / 嵌套 PROBING |
| `nogil_sleep.pyx` | nogil 相关实验片段 |
