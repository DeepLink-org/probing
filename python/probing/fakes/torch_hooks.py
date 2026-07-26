"""Monkey-patch real ``torch.distributed`` APIs to journal + dual-write.

Prefer this over MetaPath-replacing ``torch.distributed`` (which breaks an
already-imported ``torch``). Hooks wrap the live module when present.

When the process group is not initialized, collectives are **simulated**
(journal + ``comm_collective`` only) so macOS fakes loops still produce
correlatable rows without gloo/NCCL.
"""

from __future__ import annotations

import logging
import time
from typing import Any, Callable

from .journal import record_fake_event

logger = logging.getLogger(__name__)

_INSTALLED = False
_ORIGINALS: dict[str, Any] = {}

_COLLECTIVES = (
    "all_reduce",
    "broadcast",
    "all_gather",
    "reduce_scatter",
    "all_gather_into_tensor",
    "reduce_scatter_tensor",
    "barrier",
)


class _FakeWork:
    def wait(self, *args: Any, **kwargs: Any) -> None:
        del args, kwargs
        return None

    def is_completed(self) -> bool:
        return True


def _tensor_meta(tensor: Any) -> tuple[int, str, str]:
    try:
        import torch

        if isinstance(tensor, torch.Tensor):
            return (
                int(tensor.element_size() * tensor.numel()),
                str(tuple(tensor.shape)),
                str(tensor.dtype),
            )
    except Exception:
        pass
    return 0, "", ""


def _dual_write_collective(
    op: str,
    *,
    tensor: Any = None,
    duration_ms: float,
    async_op: bool = False,
) -> None:
    nbytes, shape, dtype = _tensor_meta(tensor)
    record_fake_event(
        "collective",
        op,
        nbytes=nbytes,
        duration_ms=duration_ms,
        attrs={"shape": shape, "dtype": dtype, "async_op": async_op},
    )
    try:
        import torch.distributed as dist

        from probing.profiling.collective.record import record_comm_lite

        group_rank = dist.get_rank() if dist.is_initialized() else 0
        group_size = dist.get_world_size() if dist.is_initialized() else 1
        record_comm_lite(
            op=op,
            duration_ms=duration_ms,
            group_rank=int(group_rank),
            group_size=int(group_size),
            nbytes=nbytes,
            tensor_shape=shape,
            tensor_dtype=dtype,
            async_op=async_op,
            write_trace_event=True,
        )
    except Exception as exc:
        logger.debug("comm_collective dual-write skipped: %s", exc)


def _wrap_collective(op: str, orig: Callable[..., Any]) -> Callable[..., Any]:
    def wrapped(*args: Any, **kwargs: Any) -> Any:
        import torch.distributed as dist

        tensor = args[0] if args else kwargs.get("tensor")
        async_op = bool(kwargs.get("async_op", False))

        if not dist.is_initialized():
            _dual_write_collective(
                op, tensor=tensor, duration_ms=0.0, async_op=async_op
            )
            return _FakeWork() if async_op else None

        t0 = time.perf_counter()
        result = orig(*args, **kwargs)
        duration_ms = (time.perf_counter() - t0) * 1e3
        if async_op and result is not None and hasattr(result, "wait"):
            inner_wait = result.wait

            def wait(*wargs: Any, **wkwargs: Any) -> Any:
                tw = time.perf_counter()
                out = inner_wait(*wargs, **wkwargs)
                _dual_write_collective(
                    op,
                    tensor=tensor,
                    duration_ms=(time.perf_counter() - tw) * 1e3,
                    async_op=True,
                )
                return out

            result.wait = wait  # type: ignore[method-assign]
            return result
        _dual_write_collective(
            op, tensor=tensor, duration_ms=duration_ms, async_op=False
        )
        return result

    return wrapped


def install_torch_dist_hooks() -> bool:
    """Wrap ``torch.distributed`` collectives to emit ``fake_event`` (+ comm rows)."""
    global _INSTALLED
    if _INSTALLED:
        return True
    try:
        import torch.distributed as dist
    except Exception as exc:
        logger.debug("torch.distributed unavailable for hooks: %s", exc)
        return False

    for name in _COLLECTIVES:
        orig = getattr(dist, name, None)
        if not callable(orig):
            continue
        _ORIGINALS[name] = orig
        setattr(dist, name, _wrap_collective(name, orig))

    for name in ("init_process_group", "destroy_process_group"):
        orig = getattr(dist, name, None)
        if not callable(orig):
            continue
        _ORIGINALS[name] = orig
        if name == "init_process_group":

            def make_init(o: Callable[..., Any]) -> Callable[..., Any]:
                def _wrapped(*args: Any, **kwargs: Any) -> Any:
                    result = o(*args, **kwargs)
                    record_fake_event(
                        "dist",
                        "init_process_group",
                        attrs={
                            "backend": str(kwargs.get("backend", "")),
                            "rank": str(kwargs.get("rank", "")),
                            "world_size": str(kwargs.get("world_size", "")),
                        },
                    )
                    return result

                return _wrapped

            setattr(dist, name, make_init(orig))
        else:

            def make_destroy(o: Callable[..., Any]) -> Callable[..., Any]:
                def _wrapped(*args: Any, **kwargs: Any) -> Any:
                    result = o(*args, **kwargs)
                    record_fake_event("dist", "destroy_process_group")
                    return result

                return _wrapped

            setattr(dist, name, make_destroy(orig))

    _INSTALLED = True
    record_fake_event("dist", "hooks_installed")
    logger.info("probing.fakes torch.distributed journal hooks installed")
    return True


def uninstall_torch_dist_hooks() -> None:
    global _INSTALLED
    if not _INSTALLED:
        return
    try:
        import torch.distributed as dist
    except Exception:
        _ORIGINALS.clear()
        _INSTALLED = False
        return
    for name, orig in _ORIGINALS.items():
        setattr(dist, name, orig)
    _ORIGINALS.clear()
    _INSTALLED = False
    logger.info("probing.fakes torch.distributed journal hooks removed")


def torch_dist_hooks_installed() -> bool:
    return _INSTALLED
