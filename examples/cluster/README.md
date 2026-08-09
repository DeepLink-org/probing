# Cluster

本机模拟多机 × 多卡 torchrun + 分层集群心跳。

## 入口

```bash
./examples/cluster/run_multinode.sh           # 默认 2 机 × 2 卡
./examples/cluster/run_multinode.sh 3 4       # 3 机 × 4 卡
PROBING_CLUSTER_PRESET=fast ./examples/cluster/run_multinode.sh 2 2
```

预设说明：`docs/src/design/distributed.zh.md#cluster-membership`。

## 文件

| 文件 | 说明 |
|------|------|
| `run_multinode.sh` | 编排 N 个 torchrun |
| `cluster_multinode_demo.py` | sleep + dist init |
