# Megatron

Megatron 相关示例：真实 Core 训练、fakes 本地调试、真 Megatron-LM `pretrain_gpt.py`。

## 入口

```bash
# CPU 单进程调试靶场：64 rank / 8 节点，TP2 PP4 DP8 SP2
./examples/megatron/run_64_rank_mock.sh
# browser: http://127.0.0.1:18080/training

# macOS / 无 CUDA：scripted fakes（非真 forward）
./examples/megatron/run_fakes.sh

# 真实 Megatron-LM + 底层 fake（默认 ../Megatron-LM）
./examples/megatron/run_real_lm.sh
# 切换版本：MEGATRON_LM=/path/to/other-checkout ./examples/megatron/run_real_lm.sh

# Linux + CUDA：真实 megatron-core soak + Web UI
./examples/megatron/run_soak.sh
DURATION_SEC=60 NPROC=1 TP_SIZE=1 ./examples/megatron/run_soak.sh
# browser: http://127.0.0.1:18080/
```

契约测试（mock）：`make test-python-regression` → `tests/regression/ext/test_megatron_contract.py`。

## 64-rank CPU 调试靶场

`run_64_rank_mock.sh` 通过 torchrun 启动 64 个真实 worker 进程，并将它们映射为
8 个逻辑节点、每节点 8 rank。每个 rank 绑定独立 HTTP endpoint 并自行发送
heartbeat；rank 0 不再代写其他节点。torchrun 同时提供真实 rendezvous
TCPStore，用于调试 **Cluster → Distributed Status**。它持续生成层次化
`train.step → forward / TP communication / backward / DP communication / optimizer`
span，并写入 64-rank NCCL mock 表，适合调试 Placement、趋势、timeline、stack
交互、诊断 skill 和部分节点不可达状态。

启动脚本默认将服务绑定到 `127.0.0.1:18080`，不会对外暴露调试接口；如需远程
访问，必须显式设置 `PROBING_SERVER_ADDR` 并按部署要求启用认证。

这 64 个 rank 都是可达的本地进程，但 8 个 host 名称是为 Placement 调试提供的
逻辑分组，不代表 8 台物理机器。该 fixture 仍不证明真实 Gloo/NCCL 通信；
需要真实通信行为时使用 `run_soak.sh` 或 `cluster/run_multinode.sh`。
PyTorch main/nightly 提供原生 `wait_counter_values` debug handler 时，页面优先
展示原生计数器。PyTorch 2.8 等尚未包含 `torch.distributed.debug` 的版本中，
fixture 会记录模拟 TP/DP/PP 通信段真实经过的时间，并在页面明确标记数据源为
`megatron fixture`，不会把兼容数据冒充为 PyTorch 原生计数器。

Megatron Sequence Parallel 使用 TP communication group。本例中 `SP=2` 等价于
`sp_rank = tp_rank`，因此 world size 仍为 `TP2 × PP4 × DP8 = 64`，不会再乘一次 SP。

只验证映射或导出 manifest：

```bash
PROBING=0 python examples/megatron/megatron_64_rank_mock.py --validate-only
PROBING=0 python examples/megatron/megatron_64_rank_mock.py \
  --validate-only --manifest /tmp/megatron-64-topology.json
```

## 文件

| 文件 | 说明 |
|------|------|
| `run_64_rank_mock.sh` | 启动 64-rank CPU 调试靶场 |
| `run_64_rank_worker.sh` | 在 Python 启动前把真实 torchrun rank 映射为 8×8 逻辑节点 |
| `megatron_64_rank_mock.py` | 拓扑、心跳、训练 span 与 NCCL mock |
| `run_fakes.sh` | `pretrain_gpt.py`（fakes/meta） |
| `run_real_lm.sh` | 真版 `Megatron-LM/pretrain_gpt.py` + bottom fakes |
| `run_soak.sh` | megatron-core torchrun soak |
| `pretrain_gpt.py` | Megatron 风格 CLI（fakes） |
| `run_megatron_lm_pretrain.py` | 真 LM runner |
| `megatron_meta_debug_loop.py` | meta scripted role/step loop |
| `megatron_mcore_train_loop.py` | 真实 mcore 训练循环 |

详见 `python/probing/fakes/README.md`。
