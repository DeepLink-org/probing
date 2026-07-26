# Inference

vLLM / SGLang 推理侧 probing 演示。

## 入口

```bash
# 真实 vLLM 离线推理 soak（Linux/CUDA 或 macOS vllm-metal）
./examples/inference/run_vllm_soak.sh
DURATION_SEC=60 ./examples/inference/run_vllm_soak.sh
# browser: http://127.0.0.1:18081/

# SGLang / 推理 metrics demo
./examples/inference/run_sglang_demo.sh
```

契约测试：`tests/regression/ext/test_vllm_contract.py`。

## 文件

| 文件 | 说明 |
|------|------|
| `run_vllm_soak.sh` | vLLM soak 编排 |
| `vllm_offline_soak.py` | 离线 generate 循环 |
| `run_sglang_demo.sh` | metrics demo |
| `sglang_inference_metrics_demo.py` | 推理引擎指标 |
