# 训练 Phase 语义（Tracing）

本文定义 `probing.phase()`、`train.step` 与 hook/span 协作的 **不变量**。实现见 `python/probing/tracing/phases.py`。

## 核心对象

| 概念 | 含义 |
|------|------|
| **phase** | 训练阶段枚举：`forward` / `backward` / `optimizer`（span 字段） |
| **`probing.phase()`** | 当前 span 栈上**最内层**带 training phase 的 span；无则为 `idle` |
| **`train.step`** | 分析用 span **名称**（不是 phase）；表示一次 logical iteration 的 wall time |
| **`probing.step()`** | 坐标计数器；在 **OPTIMIZER span 退出** 时 +1 `micro_step` |

## Span 命名（API spec）

```python
# 规范形式：phase 给定则 name 默认为 phase
with probing.span(phase=probing.FORWARD):
    ...

# 分析用名称（非 training phase）
with probing.span("epoch"):
    ...

# 显式 display name + phase
with probing.span("compute", phase=probing.BACKWARD):
    ...
```

`resolve_span(name, phase)` 规则：

1. 仅 `phase` → `(name=phase, phase=phase)`
2. 仅 `name` → `(name, infer(name))`
3. 两者皆有 → `(name, resolve(name, phase))`，至少其一必填

## 不变量

1. **`phase()` 来自 span 栈**，不是独立全局变量；batch 结束后显示 `idle` 是预期行为。
2. **`train.step` 起止**：从本 logical iteration 的**第一次 forward**（hook 进入）到 **optimizer hook 退出**；中间梯度累积的 forward/backward **不重置**计时器。
3. **每个 optimizer 退出**最多写一条 `train.step`（需先出现过 forward）；无 forward 的 optimizer 不写。
4. **同一 phase 同时只有一个活跃 span**：`phase_hook` 在已有同 phase span（manual / torch_probe）时不重复开 span。
5. **`micro_step`**：每次 OPTIMIZER span 退出 +1；**`local_step = micro_step // micro_batches`**（设置 `probing.step(micro_batches=k)` 对应梯度累积因子）。

## TorchProbe × phase hook（ownership）

| 能力 | phase hook | TorchProbe |
|------|------------|------------|
| iteration phase span（forward/backward/optimizer） | **拥有** | 当 `owns_training_phases(module=…)` 为真时**跳过** |
| `train.step` closed span | **拥有** | 不写 |
| 模块级 `torch_trace` 表（timing / mem） | — | **拥有** |
| 非 training 模块 span（如 init） | — | **拥有** |

检测 API：`probing.owns_training_phases(model=…)` / `optimizer=…` / `module=…`。

典型组合：

```python
probing.attach_training_phases(model, optimizer)  # iteration phase + train.step
configure("on")  # TorchProbe 仅写 torch_trace，不再开 training phase span
```

仅 TorchProbe、未 attach phase hook 时：TorchProbe 仍会开 training phase span（legacy 路径）。

## 组合规则（source）

| source | 用途 |
|--------|------|
| `manual` | 用户 `probing.span(..., phase=...)` |
| `phase_hook` | `attach_training_phases` hook |
| `torch_probe` | 模块级 TorchProbe span（training phase 可被 hook 抑制） |

同 phase 已存在活跃 span 时，hook **不再**开同名 phase span。

## 梯度累积示例

```python
probing.step(micro_batches=4)
probing.attach_training_phases(model, optimizer)

for i, batch in enumerate(loader):
    loss = model(batch) / 4
    loss.backward()
    if (i + 1) % 4 == 0:
        optimizer.step()
        optimizer.zero_grad()
```

- 每个 micro-batch：forward/backward phase span 各一对。
- 仅第 4、8、… 次 micro-batch 触发 optimizer 与 `train.step`。
- `train.step` attrs 含 `accum_index`、`micro_step`、`local_step`。

## 性能：`inspect.stack()`

自动 `location` **默认关闭**。仅在 `PROBING_SPAN_LOCATION=1` 或显式 `location=` 时，`span.py` 的 `_caller_location()` 会遍历 `inspect.stack()`。TorchProbe 变量追踪在 `torch_probe.py` 另有独立 stack  walk。
