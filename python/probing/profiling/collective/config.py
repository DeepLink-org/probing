"""Collective tracing configuration and autostart policy."""

from __future__ import annotations

import logging
import os
from dataclasses import dataclass
from typing import Optional

import probing

from probing.util.env import parse_bool_flag

from .record import CommRecordMode

logger = logging.getLogger(__name__)


def _parse_mode(raw: Optional[str]) -> CommRecordMode:
    if raw is None:
        return CommRecordMode.LITE
    normalized = str(raw).strip().lower()
    if normalized in ("full", "span", "spans"):
        return CommRecordMode.FULL
    return CommRecordMode.LITE


def is_distributed_torch_job() -> bool:
    """True when this process is part of a multi-rank torch job."""
    raw = os.environ.get("WORLD_SIZE", "1").strip()
    try:
        if int(raw) > 1:
            return True
    except ValueError:
        pass

    try:
        import torch.distributed as dist

        if dist.is_initialized() and dist.get_world_size() > 1:
            return True
    except Exception:
        pass
    return False


def nccl_profiler_plugin_active() -> bool:
    """True when the NCCL profiler plugin is configured for this process.

    The plugin collects precise, NCCL-native events into ``nccl.*`` tables;
    when it is active the Torch-API-level tracer (``python.comm_collective``,
    Python wall-clock) is redundant for timing purposes.
    """
    return bool(os.environ.get("NCCL_PROFILER_PLUGIN", "").strip())


def collective_tracing_enabled() -> bool:
    """Resolve whether Torch-side collective hooks should be installed.

    Policy:
    1. Explicit ``probing.torch.collective.enable`` always wins.
    2. If the NCCL profiler plugin is active, default **off** — the plugin's
       ``nccl.*`` tables are the precise source; keeping both on would record
       the same collectives twice with conflicting timing semantics.
    3. Otherwise, default on for multi-rank torch jobs (coarse fallback).
    """
    explicit = parse_bool_flag(
        probing.config.get_str("probing.torch.collective.enable")
    )
    if explicit is not None:
        return explicit
    if nccl_profiler_plugin_active():
        logger.info(
            "Torch-side collective tracing disabled: NCCL profiler plugin is "
            "active (nccl.* tables). Set probing.torch.collective.enable=1 "
            "to force both."
        )
        return False
    return is_distributed_torch_job()


@dataclass(frozen=True)
class CollectiveTraceConfig:
    enabled: bool
    mode: CommRecordMode = CommRecordMode.LITE
    verbose: bool = False
    cuda_sync: bool = False
    trace_file: Optional[str] = None
    resolve_group_ranks: bool = False
    trace_event: bool = True


def collective_trace_config() -> CollectiveTraceConfig:
    verbose = (
        parse_bool_flag(probing.config.get_str("probing.torch.collective.verbose"))
        or False
    )
    cuda_sync = (
        parse_bool_flag(probing.config.get_str("probing.torch.collective.sync"))
        or False
    )
    trace_file = probing.config.get_str("probing.torch.collective.trace_file")
    if trace_file is not None and not str(trace_file).strip():
        trace_file = None
    mode = _parse_mode(probing.config.get_str("probing.torch.collective.mode"))
    resolve = parse_bool_flag(
        probing.config.get_str("probing.torch.collective.resolve_ranks")
    )
    trace_event = parse_bool_flag(
        probing.config.get_str("probing.torch.collective.trace_event")
    )
    if trace_event is None:
        trace_event = True
    return CollectiveTraceConfig(
        enabled=collective_tracing_enabled(),
        mode=mode,
        verbose=verbose,
        cuda_sync=cuda_sync,
        trace_file=trace_file,
        resolve_group_ranks=resolve or (mode == CommRecordMode.FULL),
        trace_event=trace_event,
    )
