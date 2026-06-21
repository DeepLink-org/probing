## 4. Step 坐标

训练分析使用三级 step 索引，权威来源是 Rust 坐标（通过 ``probing.step`` 访问）。

| 字段 | API | 含义 |
|------|-----|------|
| `micro_step` | `probing.step.micro_step` | 最细计数；每次 ``probing.step()`` 或 ``train.step`` span 结束 +1 |
| `local_step` | `probing.step.local_step` | 训练步（每 rank）：``micro_step // micro_batches`` |
| `global_step` | `probing.step.global_step` | 与 ``local_step`` 相同（rank 对齐时即集群训练步） |
| `micro_batches` | `probing.step(micro_batches=k)` | 梯度累积倍数：每 k 个 micro_step 合成 1 个 local/global step |

```python
import probing

probing.step(micro_batches=10)   # 10 个 micro-batch → 1 个 training step
probing.step()                   # micro_step +1
probing.step(42)                 # 设置 micro_step
print(probing.step.micro_step, probing.step.local_step, probing.step.global_step)
```

SQL 表（``python.comm_collective``、``python.torch_trace``、span attributes）统一使用上述字段名。

SQL 与 skill 请用 ``local_step`` / ``global_step`` 做训练步过滤，**不要**用 ``trainer.current_step``。
