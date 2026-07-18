import logging
import os
from typing import Optional

import probing

hooks = {}


def _torch_profiling_spec() -> Optional[str]:
    """Resolve torch profiling spec from config, falling back to the env var.

    ``sync_env_settings()`` applies ``PROBING_TORCH_PROFILING`` asynchronously;
    the first ``optimizer.step()`` can run before that finishes. Reading the env
    here avoids creating a tracer without ``backward=on`` (and other flags).
    """
    spec = probing.config.get_str("probing.torch.profiling")
    if spec is not None and str(spec).strip():
        return str(spec).strip()
    env = os.environ.get("PROBING_TORCH_PROFILING", "").strip()
    if env:
        probing.config.set("probing.torch.profiling", env)
    return env or None


def is_true(value):
    if value in ["TRUE", "True", "true", "1", "YES", "Yes", "yes", "ON", "On", "on"]:
        return True
    return False


def optimizer_step_post_hook(optimizer, *args, **kwargs):
    global hooks
    from probing.tracing.hooks import maybe_auto_attach

    maybe_auto_attach(optimizer)

    if optimizer not in hooks:
        from probing.profiling.torch import install_hooks
        from probing.profiling.torch.module_utils import get_toplevel_module
        from probing.profiling.torch_probe import TorchProbe, TorchProbeConfig

        spec = _torch_profiling_spec()
        config = TorchProbeConfig.parse(spec)
        if not config.enabled:
            logging.getLogger(__name__).info(
                "Torch profiling disabled (torch.profiling=%s)",
                spec or "",
            )
            hooks[optimizer] = None
            return

        tracer = TorchProbe(config=config)
        logging.getLogger(__name__).info(
            "Torch profiling enabled: mode=%s rate=%s shadow=%s:%s backward=%s tracepy=%s sync=%s exprs=%s",
            config.mode,
            config.rate,
            config.shadow_normal,
            config.shadow_baseline if config.shadow_enabled else 0,
            config.backward,
            config.tracepy,
            config.sync,
            config.exprs or "",
        )

        try:
            models = get_toplevel_module()
            for model in models:
                install_hooks(model, tracer=tracer, backward=config.backward)
            install_hooks(opt=optimizer, tracer=tracer, backward=config.backward)
            hooks[optimizer] = tracer
        except ReferenceError as exc:
            logging.getLogger(__name__).warning(
                "Skip torch profiling hook install because a weak reference expired: %s",
                exc,
            )
            hooks[optimizer] = None
            return


def collective_hook():
    """Autostart low-overhead collective tracing for distributed torch jobs."""
    from probing.profiling.collective import maybe_start_collective_tracing

    maybe_start_collective_tracing()


def megatron_hook():
    """Autostart Megatron role/step sync when Megatron loads before torch hooks."""
    try:
        from probing.ext.megatron import maybe_autostart

        maybe_autostart()
    except Exception:
        pass


def vllm_hook():
    """Autostart vLLM role/step sync when vLLM loads before torch hooks."""
    try:
        from probing.ext.vllm import maybe_autostart

        maybe_autostart()
    except Exception:
        pass


_hook_registered = False


def init():
    global _hook_registered
    if _hook_registered:
        return
    _hook_registered = True

    from torch.optim.optimizer import register_optimizer_step_post_hook

    register_optimizer_step_post_hook(optimizer_step_post_hook)

    collective_hook()
    megatron_hook()
    vllm_hook()
    try:
        from probing.crash import install

        install()
    except Exception:
        pass


def deinit():
    from probing.profiling.torch import uninstall_hooks

    uninstall_hooks()
