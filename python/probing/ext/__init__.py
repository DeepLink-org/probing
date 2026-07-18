"""
Framework Extensions

Spec
----
This package provides integrations for third-party AI frameworks.

Responsibilities:
1.  Provide automatic tracing for distributed frameworks (e.g., Ray).
2.  Provide hooks and profilers for Deep Learning frameworks (e.g., PyTorch).
3.  Normalize framework-specific events into Probing spans.

Submodules:
- `ray`: Ray task and actor tracing (framework-neutral identity via PROBING_* env).
- `torch`: PyTorch profiling hooks and utilities.
- `megatron`: Megatron-LM role/step sync.
- `vllm`: vLLM / vLLM-Metal inference role/step sync.
- `engines`: Inference-engine Prometheus metrics adapters (e.g. SGLang) for agentic RL.
- `slime`: Slime adapter — cmdline/env → neutral process roles; Slime router registration.
"""
