# Examples

按主题分目录；每个目录有 `README.md` 和可执行 shell 入口。

| 目录 | 主题 | 快速入口 |
|------|------|----------|
| [getting-started/](getting-started/) | Tracing / hooks / 冒烟 | `./examples/getting-started/run_tracing.sh` |
| [megatron/](megatron/) | Megatron-Core / fakes / 真 LM | `./examples/megatron/run_fakes.sh` |
| [imagenet/](imagenet/) | ImageNet soak / DDP | `./examples/imagenet/run_soak.sh` |
| [inference/](inference/) | vLLM / SGLang | `./examples/inference/run_vllm_soak.sh` |
| [ray/](ray/) | Ray tracing / actor spans | `./examples/ray/run_tracing.sh` |
| [crash/](crash/) | Crash 捕获 | `./examples/crash/run_demo.sh` |
| [cluster/](cluster/) | 多机 torchrun 集群 | `./examples/cluster/run_multinode.sh` |
| [overhead/](overhead/) | 开销基准 / NCCL | `./examples/overhead/run_bench.sh` |
| [job-tracker/](job-tracker/) | 作业生命周期 hook | `./examples/job-tracker/run.sh` |
| [probing-acme/](probing-acme/) | 厂商扩展包模板 | 见目录 README |

框架集成 **契约测试** 在 `tests/regression/ext/`（`make test-python-regression`）。

## 依赖

```bash
source .venv/bin/activate   # after make develop
uv pip install torch        # 多数示例
# ImageNet: uv pip install torchvision
```

## 常用命令

```bash
./examples/getting-started/run_tracing.sh
./examples/megatron/run_fakes.sh
./examples/megatron/run_real_lm.sh          # 需 ../Megatron-LM
DURATION_SEC=60 ./examples/imagenet/run_soak.sh
make bench-quick                            # → examples/overhead/bench_instrumentation.py
make soak-quick                             # → examples/imagenet/run_soak.sh
```

## 更多文档

- [Examples (MkDocs)](../docs/src/examples/index.md)
- [Quick Start](../docs/src/quickstart.md)
- [Vendor extension template](probing-acme/)
