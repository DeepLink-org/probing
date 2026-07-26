# ImageNet / soak

合成 ImageNet 训练长跑、DDP 演示与 soak 断言（CI `soak.yml` 使用本目录）。

## 入口

```bash
# ~10 min 单进程 CPU soak + assertions
./examples/imagenet/run_soak.sh

# 短冒烟
DURATION_SEC=60 MAX_STEPS=8 ./examples/imagenet/run_soak.sh

# 2-rank gloo DDP
NPROC=2 DIST_BACKEND=gloo DURATION_SEC=120 ./examples/imagenet/run_soak.sh

# 专用分布式 demo（Web UI :18080）
./examples/imagenet/run_ddp.sh
```

## 依赖

`torch` + `torchvision`

## 文件

| 文件 | 说明 |
|------|------|
| `run_soak.sh` | 长跑编排（CI / `make soak`） |
| `run_ddp.sh` | 默认 2-rank DDP |
| `imagenet_with_span.py` | 带 span 的训练脚本 |
| `imagenet.py` | 经典 ImageNet 训练 |
| `soak_assert.py` | 训后 SQL / overhead 断言 |
