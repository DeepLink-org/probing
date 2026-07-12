#!/usr/bin/env python3
"""
vLLM + vLLM-Metal + probing 离线推理 soak
========================================

使用真实 vLLM ``LLM.generate`` 长跑，写入 tracing / torch 表，
配合 ``PROBING_PORT`` 在浏览器打开 Web UI。

* Linux + CUDA：``uv pip install vllm``（或按官方文档安装）
* macOS Apple Silicon：安装 [vLLM-Metal](https://github.com/vllm-project/vllm-metal)

契约测试（mock）：``tests/regression/ext/test_vllm_contract.py`` — ``make test-python-regression``

运行::

    ./examples/run_vllm_soak.sh
    VLLM_MODEL=facebook/opt-125m DURATION_SEC=120 ./examples/run_vllm_soak.sh
"""

from __future__ import annotations

import argparse
import os
import sys
import time

import probing
from probing.ext.vllm import maybe_autostart, sync_role_from_env, sync_step_from_llm

try:
    from vllm import LLM, SamplingParams
except ImportError as exc:  # pragma: no cover - optional heavy dep
    raise SystemExit(
        "vLLM is required for this example.\n"
        "  Linux/CUDA: uv pip install vllm\n"
        "  macOS: install vllm-metal — https://github.com/vllm-project/vllm-metal"
    ) from exc


_DEFAULT_PROMPTS = (
    "Hello, my name is",
    "The capital of France is",
    "Probing observes inference with vLLM",
    "A short poem about GPUs:",
)


def _default_model() -> str:
    if sys.platform == "darwin":
        return "mlx-community/Qwen2.5-0.5B-Instruct-4bit"
    return "facebook/opt-125m"


def _default_prompts_per_batch() -> int:
    # vllm-metal batched decode needs mlx_lm.BatchKVCache.merge (not in all mlx-lm
    # releases). Single-prompt batches use the sequential decode path and work reliably.
    if sys.platform == "darwin":
        return 1
    return len(_DEFAULT_PROMPTS)


def _prompts_for_batch(
    prompts: list[str], *, batch_index: int, batch_size: int
) -> list[str]:
    if batch_size <= 1:
        return [prompts[batch_index % len(prompts)]]
    start = (batch_index * batch_size) % len(prompts)
    out: list[str] = []
    for offset in range(batch_size):
        out.append(prompts[(start + offset) % len(prompts)])
    return out


def _wait_for_server_address(*, timeout_sec: float = 5.0) -> str | None:
    deadline = time.monotonic() + timeout_sec
    while time.monotonic() < deadline:
        addr = probing.config.get_str("server.address")
        if addr and str(addr).strip():
            return str(addr).strip().strip("'\"")
        time.sleep(0.1)
    return None


def _print_observability_hints() -> None:
    pid = os.getpid()
    addr = _wait_for_server_address()
    print(f"pid={pid} probing={'on' if probing.is_enabled() else 'off'}")
    if addr:
        print(f"Web UI:  http://{addr}/")
        print(f"         http://{addr}/investigate")
        print(
            f'CLI:     probing -t {addr} query '
            f'"SELECT local_step, name FROM python.trace_event LIMIT 8"'
        )
    else:
        print(f'CLI:     probing -t {pid} query "SELECT local_step FROM python.trace_event LIMIT 8"')
        print("Tip:     set PROBING_PORT=18081 (see run_vllm_soak.sh) for browser Web UI.")
    print(flush=True)


class SoakLimits:
    def __init__(self, *, max_batches: int = 0, max_duration_sec: int = 0) -> None:
        self.max_batches = max(0, max_batches)
        self.max_duration_sec = max(0, max_duration_sec)
        self.batch_count = 0
        self.started_at = time.monotonic()

    @property
    def enabled(self) -> bool:
        return self.max_batches > 0 or self.max_duration_sec > 0

    def tick(self) -> None:
        self.batch_count += 1

    def should_stop(self) -> bool:
        if self.max_batches > 0 and self.batch_count >= self.max_batches:
            return True
        if self.max_duration_sec > 0:
            return (time.monotonic() - self.started_at) >= self.max_duration_sec
        return False

    def stop_reason(self) -> str:
        if self.max_batches > 0 and self.batch_count >= self.max_batches:
            return f"max_batches={self.max_batches}"
        if self.max_duration_sec > 0:
            elapsed = time.monotonic() - self.started_at
            if elapsed >= self.max_duration_sec:
                return (
                    f"max_duration_sec={self.max_duration_sec} (elapsed={elapsed:.1f}s)"
                )
        return "unknown"


def main() -> None:
    parser = argparse.ArgumentParser(description="vLLM offline inference + probing soak")
    parser.add_argument(
        "--model",
        default=os.environ.get("VLLM_MODEL", _default_model()),
        help="HF model id (macOS: mlx-community/*; Linux: e.g. facebook/opt-125m)",
    )
    parser.add_argument("--max-duration-sec", type=int, default=0)
    parser.add_argument("--max-batches", type=int, default=0)
    parser.add_argument("--batch-sleep-ms", type=int, default=0)
    parser.add_argument("--print-freq", type=int, default=5)
    parser.add_argument("--max-tokens", type=int, default=32)
    parser.add_argument("--max-model-len", type=int, default=512)
    parser.add_argument("--temperature", type=float, default=0.0)
    parser.add_argument(
        "--prompts-per-batch",
        type=int,
        default=int(os.environ.get("VLLM_PROMPTS_PER_BATCH", _default_prompts_per_batch())),
        help="Prompts per llm.generate() call (macOS default 1: vllm-metal batched decode "
        "needs mlx_lm.BatchKVCache.merge)",
    )
    args = parser.parse_args()
    if args.prompts_per_batch < 1:
        raise SystemExit("--prompts-per-batch must be >= 1")

    soak = SoakLimits(
        max_batches=args.max_batches,
        max_duration_sec=args.max_duration_sec,
    )

    print(f"model={args.model}")
    print(f"prompts_per_batch={args.prompts_per_batch}")
    if soak.enabled:
        print(
            f"soak limits: max_batches={soak.max_batches or '∞'}  "
            f"max_duration_sec={soak.max_duration_sec or '∞'}"
        )
    _print_observability_hints()

    # vLLM import may occur after probing; ensure hooks are applied.
    maybe_autostart()

    with probing.span("vllm_load"):
        llm = LLM(
            model=args.model,
            max_model_len=args.max_model_len,
            trust_remote_code=True,
        )

    maybe_autostart()
    sync_role_from_env()
    print(f"role={probing.current_role()!r}", flush=True)

    sampling_params = SamplingParams(
        temperature=args.temperature,
        max_tokens=args.max_tokens,
    )
    prompts = list(_DEFAULT_PROMPTS)

    try:
        while True:
            if soak.enabled and soak.should_stop():
                break

            with probing.span("generate_batch"):
                batch_idx = soak.batch_count
                batch_prompts = _prompts_for_batch(
                    prompts,
                    batch_index=batch_idx,
                    batch_size=args.prompts_per_batch,
                )
                probing.event("batch.start", attributes=[{"batch": batch_idx}])
                outputs = llm.generate(batch_prompts, sampling_params)
                sync_step_from_llm(llm, force=True)
                sync_role_from_env()
                soak.tick()

                if soak.batch_count % args.print_freq == 0:
                    snap = probing.step.snapshot()
                    sample = outputs[0].outputs[0].text if outputs else ""
                    print(
                        f"batch={soak.batch_count} local_step={snap.local_step} "
                        f"sample={sample[:80]!r}",
                        flush=True,
                    )

                probing.event(
                    "batch.end",
                    attributes=[
                        {"batch": batch_idx},
                        {"requests": len(outputs)},
                    ],
                )

            if args.batch_sleep_ms > 0:
                time.sleep(args.batch_sleep_ms / 1000.0)

    except KeyboardInterrupt:
        print("\n=> interrupted", flush=True)

    if soak.enabled:
        print(f"=> soak stop: {soak.stop_reason()}", flush=True)

    print(f"finished after {soak.batch_count} generate batch(es)")
    _print_observability_hints()


if __name__ == "__main__":
    main()
