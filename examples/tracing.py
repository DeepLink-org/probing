#!/usr/bin/env python3
"""
probing tracing 入门
===================

两层 API，分工明确：

     ① ``attach_training_phases(model, optimizer)`` — **推荐，零侵入**
     model / optimizer 上的 hook 自动追踪每个 batch 的
     forward → backward → optimizer，并记录 ``train.step`` 整步耗时。
     训练循环里 **不需要** ``with probing.span("forward")``。

     梯度累积：先 ``probing.step(micro_batches=N)``，``train.step`` 覆盖
     N 个 micro-batch 的 wall time；详见 ``docs/src/design/training-phase.zh.md``。

  ② ``probing.span`` / ``probing.event`` — **可选，粗粒度时间线**
     包住模型初始化、epoch 等；与 ① 的 phase span 互不冲突。

运行::

    PROBING=1 python examples/tracing.py

终端实时查看 span（与 memtable 同时生效）::

    PROBING=1 PROBING_SPAN_BACKENDS=memtable,logger python examples/tracing.py

查看最近 span（另开终端）::

    probing -t <pid> query "
      SELECT s.name, s.phase,
             round((e.time - s.time) / 1e6, 2) AS ms
      FROM python.trace_event s
      JOIN python.trace_event e
        ON s.span_id = e.span_id AND e.record_type = 'span_end'
      WHERE s.record_type = 'span_start'
      ORDER BY s.time DESC LIMIT 12"
"""

from __future__ import annotations

import os

import probing
import torch
import torch.nn as nn
import torch.nn.functional as F

# --- 尽量小：只依赖 torch，无需真实数据集 ---------------------------------

DEVICE = torch.device("cuda" if torch.cuda.is_available() else "cpu")
BATCHES = 5
BATCH_SIZE = 16
LR = 0.01


class TinyNet(nn.Module):
    def __init__(self) -> None:
        super().__init__()
        self.fc1 = nn.Linear(8, 16)
        self.fc2 = nn.Linear(16, 4)

    def forward(self, x: torch.Tensor) -> torch.Tensor:
        return self.fc2(F.relu(self.fc1(x)))


def train_one_batch(
    model: nn.Module,
    optimizer: torch.optim.Optimizer,
    batch_idx: int,
) -> float:
    x = torch.randn(BATCH_SIZE, 8, device=DEVICE)
    y = torch.randint(0, 4, (BATCH_SIZE,), device=DEVICE)

    logits = model(x)
    loss = F.cross_entropy(logits, y)

    optimizer.zero_grad()
    loss.backward()
    optimizer.step()  # hook 在此追踪 optimizer phase，并推进 probing.step()

    # 可选：在 hook 打开的 phase span 上挂 point event
    if batch_idx == 0:
        probing.event(
            "batch.stats",
            attributes=[
                {"loss": round(float(loss.item()), 4)},
                {"phase": probing.phase()},
            ],
        )

    return float(loss.item())


def main() -> None:
    pid = os.getpid()
    print(
        f"pid={pid}  device={DEVICE}  probing={'on' if probing.is_enabled() else 'off'}"
    )
    print()

    # --- ② 粗粒度 span：初始化 -----------------------------------------------
    with probing.span("setup"):
        model = TinyNet().to(DEVICE)
        optimizer = torch.optim.SGD(model.parameters(), lr=LR)

    # --- ① 自动 phase span：一行挂载 -----------------------------------------
    probing.attach_training_phases(model, optimizer)
    print("attach_training_phases ✓  （forward / backward / optimizer 由 hook 驱动）")
    print()

    # --- 训练：循环内无需手写 phase span -------------------------------------
    with probing.span("epoch"):
        for i in range(BATCHES):
            loss = train_one_batch(model, optimizer, i)
            print(
                f"  batch {i}  loss={loss:.4f}  phase={probing.phase()!r}  step={probing.step.micro_step}"
            )

    print()
    if probing.is_enabled():
        print("完成。hook 已写入 python.trace_event，可用上方 SQL 查询。")
        print(
            f'  probing -t {pid} query "SELECT name, phase FROM python.trace_event LIMIT 12"'
        )
    else:
        print("完成（未落表）。请用 PROBING=1 重新运行。")


if __name__ == "__main__":
    main()
