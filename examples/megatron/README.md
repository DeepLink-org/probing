# Megatron

Megatron 相关示例：真实 Core 训练、fakes 本地调试、真 Megatron-LM `pretrain_gpt.py`。

## 入口

```bash
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

## 文件

| 文件 | 说明 |
|------|------|
| `run_fakes.sh` | `pretrain_gpt.py`（fakes/meta） |
| `run_real_lm.sh` | 真版 `Megatron-LM/pretrain_gpt.py` + bottom fakes |
| `run_soak.sh` | megatron-core torchrun soak |
| `pretrain_gpt.py` | Megatron 风格 CLI（fakes） |
| `run_megatron_lm_pretrain.py` | 真 LM runner |
| `megatron_meta_debug_loop.py` | meta scripted role/step loop |
| `megatron_mcore_train_loop.py` | 真实 mcore 训练循环 |

详见 `python/probing/fakes/README.md`。
