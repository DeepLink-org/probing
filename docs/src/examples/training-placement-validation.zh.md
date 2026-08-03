# 验证 64-rank 训练 Placement

该验证使用 CPU mock 的 Megatron 拓扑，检查 Training 页面的 Placement
数据模型与交互。它**不验证**真实 GPU 执行、NCCL 传输、带宽或 collective
延迟。

启动器使用 `torchrun` 启动 64 个真实本地 worker 进程。每个 rank 都绑定
独立 HTTP endpoint 并自行发送 heartbeat。8 个 host 名称是 Placement 的逻辑
分组，不代表 8 台物理机器。torchrun 也会向 **Cluster → Distributed
Status** 提供真实 rendezvous TCPStore。Wait Counter 不会被伪造；只有
当前 PyTorch 构建注册了实验性 `wait_counter_values` handler 时才可用。

## 验证配置

| 配置项 | 数值 |
|---|---:|
| 机器数 | 8 |
| 每台机器进程数 | 8 |
| World size | 64 |
| Tensor Parallel（TP） | 2 |
| Pipeline Parallel（PP） | 4 |
| Data Parallel（DP） | 8 |

rank 与并行坐标使用以下确定性映射：

```text
rank = dp * (PP * TP) + pp * TP + tp
tp = rank % 2
pp = (rank // 2) % 4
dp = rank // 8
host = rank // 8
local_rank = rank % 8
```

因此 rank 0 的坐标为 `D0 P0 T0`，预期通信组为：

- TP：rank `0, 1`，共 2 个 rank
- PP：rank `0, 2, 4, 6`，共 4 个 rank
- DP：rank `0, 8, 16, 24, 32, 40, 48, 56`，共 8 个 rank

## 验证证据

节点 API 返回 64 条记录、8 个不同的 host 和 64 个不同的并行角色；边界角色为
`rank 0 = dp=0,pp=0,tp=0`、`rank 63 = dp=7,pp=3,tp=1`。

页面摘要显示 `8 hosts`、`64 / 64 ranks`、`DP8`、`PP4`、`TP2`。
截图中选中了 rank 0。渲染状态中有 1 个焦点方格、1 个额外 TP 方格、
3 个额外 PP 方格和 7 个额外 DP 方格；加上焦点方格后，通信组大小分别为
2、4、8。

![64-rank、TP2、PP4、DP8 的 Training Placement](../assets/screenshots/training-placement-64-ranks-tp2-pp4-dp8.jpg)

## 自动化检查

Web 单元测试构造相同的 64 个节点，校验机器、rank 和并行维度推导；fake
runtime 单元测试让全部 64 组坐标通过 CPU Megatron mock，并校验角色键互不重复。

```bash
cd web
cargo test placement_summarizes_64_rank_megatron_topology

cd ..
PROBING=0 .venv/bin/pytest \
  tests/unit/probing/fakes/test_fakes.py \
  -k 64_rank_tp2_pp4_dp8 -q
```

三类证据覆盖不同边界：Python 测试检查角色生成，Rust 测试检查 Placement
推导与通信组归属，浏览器检查覆盖最终渲染和交互。
