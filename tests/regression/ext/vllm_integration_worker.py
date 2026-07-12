"""Subprocess worker for vLLM integration tests.

Runs a minimal offline ``LLM.generate`` when a model id is provided via
``PROBING_VLLM_TEST_MODEL``. Otherwise only validates import-hook wiring.
"""

from __future__ import annotations

import json
import os
import sys


def main() -> int:
    os.environ.setdefault("PROBING", "1")
    os.environ.setdefault("PROBING_VLLM", "on")
    os.environ.setdefault("PROBING_VLLM_STEP_SYNC", "on")
    os.environ.setdefault("VLLM_USE_V1", "1")

    if sys.platform == "darwin":
        os.environ.setdefault("VLLM_METAL_USE_MLX", "1")
        os.environ.setdefault("VLLM_MLX_DEVICE", "gpu")
        os.environ.setdefault("VLLM_WORKER_MULTIPROC_METHOD", "spawn")
        os.environ.setdefault("VLLM_PROMPTS_PER_BATCH", "1")

    import importlib

    vllm = importlib.import_module("vllm")

    import probing
    from probing.ext import vllm as vllm_ext

    for mod_name in (
        "vllm_metal",
        "vllm.v1.engine.llm_engine",
        "vllm.engine.llm_engine",
    ):
        try:
            importlib.import_module(mod_name)
        except ImportError:
            continue

    vllm_ext.maybe_autostart()

    engine_wrapped = False
    for mod_name in ("vllm.v1.engine.llm_engine", "vllm.engine.llm_engine"):
        mod = sys.modules.get(mod_name)
        if mod is None:
            continue
        engine_cls = getattr(mod, "LLMEngine", None)
        if engine_cls is not None and getattr(
            engine_cls.step, "_probing_wrapped", False
        ):
            engine_wrapped = True
            break

    result: dict[str, object] = {
        "role": probing.current_role(),
        "engine_wrapped": engine_wrapped,
        "vllm_version": getattr(vllm, "__version__", "unknown"),
    }

    model = os.environ.get("PROBING_VLLM_TEST_MODEL", "").strip()
    if model:
        from probing.ext.vllm import sync_step_from_llm

        llm = vllm.LLM(model=model, max_model_len=512)
        llm.generate(["ping"], max_tokens=1)
        sync_step_from_llm(llm, force=True)
        snap = probing.step.snapshot()
        result["local_step"] = int(snap.local_step)
        result["model"] = model

    print(json.dumps(result))
    return 0


if __name__ == "__main__":
    sys.exit(main())
