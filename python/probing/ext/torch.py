import logging

import probing

hooks = {}


def is_true(value):
    if value in ["TRUE", "True", "true", "1", "YES", "Yes", "yes", "ON", "On", "on"]:
        return True
    return False


def optimizer_step_post_hook(optimizer, *args, **kwargs):
    global hooks
    if optimizer not in hooks:
        from probing.profiling.torch import install_hooks
        from probing.profiling.torch.module_utils import get_toplevel_module
        from probing.profiling.torch_probe import TorchProbe, TorchProbeConfig

        spec = probing.config.get_str("probing.torch.profiling")

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
            "Torch profiling enabled: mode=%s rate=%s tracepy=%s sync=%s exprs=%s",
            config.mode,
            config.rate,
            config.tracepy,
            config.sync,
            config.exprs or "",
        )

        models = get_toplevel_module()
        for model in models:
            install_hooks(model, tracer=tracer)
        install_hooks(opt=optimizer, tracer=tracer)
        hooks[optimizer] = tracer


def collective_hook():
    """Autostart low-overhead collective tracing for distributed torch jobs."""
    from probing.profiling.collective import maybe_start_collective_tracing

    maybe_start_collective_tracing()


_hook_registered = False
_dist_init_patched = False


def _patch_dist_init_process_group() -> None:
    global _dist_init_patched
    if _dist_init_patched:
        return
    try:
        import torch.distributed as dist
    except ImportError:
        return

    _dist_init_patched = True
    original = dist.init_process_group

    def init_process_group(*args, **kwargs):
        original(*args, **kwargs)
        try:
            from probing.torchrun_cluster import maybe_setup_torchrun_cluster

            maybe_setup_torchrun_cluster()
        except Exception as exc:
            logging.getLogger(__name__).debug(
                "probing torchrun cluster setup skipped: %s", exc
            )

    dist.init_process_group = init_process_group  # type: ignore[assignment]


def init():
    global _hook_registered
    if _hook_registered:
        return
    _hook_registered = True

    from torch.optim.optimizer import register_optimizer_step_post_hook

    register_optimizer_step_post_hook(optimizer_step_post_hook)

    _patch_dist_init_process_group()
    collective_hook()


def deinit():
    from probing.profiling.torch import uninstall_hooks

    uninstall_hooks()
