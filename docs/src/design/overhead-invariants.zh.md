# TorchProbe Overhead 不变量（Agent / 维护者必读）

本文档列出 **不可随意改动的语义与公式**。修改 `web/src/overhead/`、`python/probing/profiling/torch_probe.py`、`python/probing/profiling/deferred_drain.py` 或 `skills/health_overview/steps.yaml` 中与 overhead 相关的逻辑前，请先读本文并更新对应测试。

完整背景见 [overhead.zh.md](overhead.zh.md)。

---

## 1. 核心不变量

### I1 — 主告警与 UI 主数字用 median，不用 mean 比值

| 指标 | 公式 | 禁止替代 |
|------|------|----------|
| `dispatch_overhead_pct` | `median(dispatch) ÷ median(shadow) − 1` | `mean(dispatch) ÷ mean(shadow)` |
| `blended_overhead_pct` | `median(all probed) ÷ median(shadow) − 1` | 混合 mean |
| `sampled_overhead_pct` | `median(sampled) ÷ median(shadow) − 1` | — |

**原因**：训练步墙钟抖动大、`shadow_n` 少时，mean 比值会与 median 差一个数量级（例如 median ≈2% 而 mean 比值 ≈20%+），会误导用户。

**守护测试**：`web/src/overhead/metrics.rs` → `amortized_not_mean_ratio_when_means_diverge`

---

### I2 — Amortized（Effective overhead）= 采样率加权，不是 mean 摊销

```
amortized = (1 − rate) × dispatch_overhead + rate × sampled_overhead
```

- `rate`：配置 `sample_rate`，否则 `sampled_n / probed_n`
- 无采样步时：`amortized == dispatch_overhead`

**禁止**：`mean(probed) ÷ mean(shadow) − 1` 作为 Web UI 的 Amortized / Effective overhead。

**守护测试**：`amortized_blends_dispatch_and_sampled_by_rate`、`amortized_not_mean_ratio_when_means_diverge`

---

### I3 — `step_duration_sec` 记时边界

在 `TorchProbe.post_step_hook` / `_close_step_wall` 中，顺序必须为：

```
_record_step_timing()      # 墙钟终点
_drain_deferred()          # deferred 回收（可在后台线程执行 save）
_advance_step_cycle_for_next()
_mark_step_wall_start()    # 下一步起点
```

**禁止**：在 `_record_step_timing()` **之前**调用 `_drain_deferred()`（会把前几步 event 回收算进本步墙钟）。

**守护测试**：`tests/regression/profiling/test_torch_probe_sampling.py::test_post_step_hook_drains_deferred_after_step_timing`、`test_overhead_invariants.py::test_close_step_wall_source_order`

---

### I4 — Deferred 回收默认异步

- 默认 `PROBING_TORCH_DEFER_ASYNC=1`：ready 的 `DelayedRecord` 入队，后台线程 `elapsed_time` + `save()`
- 队列满 → 主线程同步 `save()` 回退（不丢数据）
- 进程退出 `atexit` flush

**禁止**：在无测试、无文档的情况下改回「仅在训练线程同步 drain」作为唯一路径。

**守护测试**：`tests/regression/profiling/test_deferred_drain_worker.py`

---

### I5 — 稳定性门控

百分比在 UI 上视为「稳定」需同时满足：

- `shadow_baseline > 0`
- `shadow_n ≥ 5`（`MIN_SHADOW_SAMPLES`）
- `dispatch_n ≥ 16`（`MIN_DISPATCH_SAMPLES`）

`dispatch_overhead_pct` / `blended_overhead_pct` 在不稳定时不应展示为精确告警数字（可为 `—` 或 muted）。

**守护测试**：`snapshot_computes_dispatch_overhead`、`skills/health_overview` SQL 中的 `dispatch_n` / `shadow_n`

---

### I6 — 展示语义（L4 Web）

| 规则 | 说明 |
|------|------|
| 低开销显示 | `|pct| < 0.5%` → `≈0%`；`< 5%` → `~N%`（避免 `+1.9%` 告警感） |
| 主指标命名 | UI 主卡：「Typical overhead」= dispatch；「Effective overhead」= amortized |
| 训练日志对齐 | `torch_step_timing` 为 hook-to-hook 墙钟，含 DataLoader 等待；不等于仅 compute 的 `time=49ms` 打印 |

**守护测试**：`format_pct_signed_*`、`sidebar_copy_when_stable`

---

## 2. 测试地图

| 文件 | 守护的不变量 |
|------|----------------|
| `web/src/overhead/metrics.rs` (`#[cfg(test)]`) | I1, I2, I5, I6 |
| `tests/regression/profiling/test_overhead_invariants.py` | I3, I4（源码顺序 / 默认 env） |
| `tests/regression/profiling/test_torch_probe_sampling.py` | I3, deferred settle 窗口 |
| `tests/regression/profiling/test_deferred_drain_worker.py` | I4 |
| `skills/health_overview/steps.yaml` | I1 告警列 `dispatch_overhead_pct` |

本地命令：

```bash
# Rust Web 指标
cd web && cargo test overhead

# Python hook / drain
PROBING=0 pytest tests/regression/profiling/ -q
```

---

## 3. Agent 修改前检查清单

1. 是否改动 overhead **公式**？→ 更新本文 §1 + `metrics.rs` 测试 + `overhead.zh.md`
2. 是否改动 **hook 顺序**？→ 更新 `test_post_step_hook_drains_deferred_after_step_timing` 与 `test_close_step_wall_source_order`
3. 是否改动 **异步 drain** 默认？→ 更新 `deferred_drain.py` 测试与 §I4
4. 是否只改 UI 文案？→ 保持 I6；跑 `cargo test -p web overhead`
5. **不要** 用 mean 比值「修复」amortized 与 median 不一致 — 那是预期行为

---

## 4. 参考场景（回归夹具）

用户实测（median 一致、mean 失真）：

| 观测 | 值 |
|------|-----|
| shadow median | 180 ms |
| dispatch median | 166 ms |
| dispatch overhead | ≈ +1.9% |
| shadow mean | 130 ms |
| probed mean | 533 ms |
| mean 比值（禁止作 amortized） | ≫ 20% |
| 期望 amortized（rate≈5%） | ≈ +1.5% ~ +2% |

此场景编码在 `amortized_not_mean_ratio_when_means_diverge` 测试中。
