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

        from probing.profiling.torch import next_step

        next_step()


def collective_hook():
    """Autostart low-overhead collective tracing for distributed torch jobs."""
    from probing.profiling.collective import maybe_start_collective_tracing

    maybe_start_collective_tracing()


_hook_registered = False


def init():
    global _hook_registered
    if _hook_registered:
        return
    _hook_registered = True

    from torch.optim.optimizer import register_optimizer_step_post_hook

    register_optimizer_step_post_hook(optimizer_step_post_hook)

    collective_hook()


def deinit():
    from probing.profiling.torch import uninstall_hooks

    uninstall_hooks()
