__ALL__ = [
    "trace_all_collectives",
    "collective_tracing_enabled",
    "collective_trace_config",
]

from .coll import CollectiveTracer
from .config import collective_trace_config, collective_tracing_enabled
from .record import CommRecordMode

_installed_tracer: CollectiveTracer | None = None


def trace_all_collectives(
    trace_file=None,
    verbose=False,
    *,
    cuda_sync=False,
    mode: CommRecordMode = CommRecordMode.LITE,
    resolve_group_ranks: bool = False,
    trace_event: bool = True,
):
    """Install hooks on ``torch.distributed`` collectives."""
    global _installed_tracer
    if _installed_tracer is not None:
        return _installed_tracer

    tracer = CollectiveTracer(
        trace_file=trace_file,
        verbose=verbose,
        cuda_sync=cuda_sync,
        mode=mode,
        resolve_group_ranks=resolve_group_ranks,
        trace_event=trace_event,
    )
    tracer.apply_hooks()
    _installed_tracer = tracer
    return tracer


def maybe_start_collective_tracing() -> CollectiveTracer | None:
    """Autostart collective tracing when policy allows."""
    cfg = collective_trace_config()
    if not cfg.enabled:
        return None
    tracer = trace_all_collectives(
        trace_file=cfg.trace_file,
        verbose=cfg.verbose,
        cuda_sync=cfg.cuda_sync,
        mode=cfg.mode,
        resolve_group_ranks=cfg.resolve_group_ranks,
        trace_event=cfg.trace_event,
    )
    import logging

    logging.getLogger(__name__).info(
        "Collective tracing enabled (mode=%s, cuda_sync=%s, verbose=%s)",
        cfg.mode.value,
        cfg.cuda_sync,
        cfg.verbose,
    )
    return tracer
