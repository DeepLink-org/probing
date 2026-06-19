"""Collective tracing configuration and autostart policy."""

from __future__ import annotations

import logging
import os
from dataclasses import dataclass
from typing import Optional

import probing

from .record import CommRecordMode

logger = logging.getLogger(__name__)

_TRUE = {"1", "true", "yes", "on", "enable", "enabled"}
_FALSE = {"0", "false", "no", "off", "disable", "disabled"}


def _flag(value: Optional[str]) -> Optional[bool]:
    if value is None:
        return None
    normalized = str(value).strip().lower()
    if normalized in _TRUE:
        return True
    if normalized in _FALSE:
        return False
    return None


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


def collective_tracing_enabled() -> bool:
    """Resolve whether collective hooks should be installed."""
    explicit = _flag(probing.config.get_str("probing.torch.collective.enable"))
    if explicit is not None:
        return explicit
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
    verbose = _flag(probing.config.get_str("probing.torch.collective.verbose")) or False
    cuda_sync = _flag(probing.config.get_str("probing.torch.collective.sync")) or False
    trace_file = probing.config.get_str("probing.torch.collective.trace_file")
    if trace_file is not None and not str(trace_file).strip():
        trace_file = None
    mode = _parse_mode(probing.config.get_str("probing.torch.collective.mode"))
    resolve = _flag(probing.config.get_str("probing.torch.collective.resolve_ranks"))
    trace_event = _flag(probing.config.get_str("probing.torch.collective.trace_event"))
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
