"""Torch-API-level collective tracer (legacy, coarse).

Monkey-patches ``torch.distributed`` collectives and measures **Python
wall-clock around the API call** (for ``async_op`` the timing closes at
``work.wait()``). This is *launch/API* timing — not NCCL execution time.

For precise, NCCL-native data (kernel/proxy-reconstructed execution time,
wait decomposition, bandwidth) use the NCCL profiler plugin's ``nccl.*``
tables instead; this tracer is the fallback when the plugin is unavailable
and is disabled by default when the plugin is active (see ``config.py``).
"""

import logging
import os
import time
from functools import wraps
from typing import List, Optional, Tuple

import torch
import torch.distributed as dist

from .record import (
    CommRecordMode,
    begin_comm_span,
    finish_comm_span,
    record_comm_lite,
)

logger = logging.getLogger(__name__)

function_names = [
    "all_reduce",
    "all_gather",
    "reduce_scatter",
    "broadcast",
    "reduce_scatter_base",
    "all_gather_base",
    "reduce_scatter_tensor",
    "all_gather_into_tensor",
]

GROUP_RANKS_CACHE = {}


def get_participating_ranks(
    group: Optional[dist.ProcessGroup] = None,
) -> Tuple[int, int, List[int]]:
    if not dist.is_initialized():
        return 0, 0, []

    group_rank = dist.get_rank(group=group)
    group_size = dist.get_world_size(group=group)

    if group is None or group == dist.group.WORLD:
        return group_rank, group_size, list(range(dist.get_world_size()))

    group_id = id(group)

    if group_id in GROUP_RANKS_CACHE:
        return group_rank, group_size, GROUP_RANKS_CACHE[group_id]

    try:
        ranks_list = [None] * group_size
        global_rank = dist.get_rank()
        dist.all_gather_object(ranks_list, global_rank, group=group)
        ranks = [int(r) for r in ranks_list]
        GROUP_RANKS_CACHE[group_id] = ranks
        return group_rank, group_size, ranks

    except Exception as e:
        logger.warning(
            "[rank %s] all_gather_object failed: %s; using TCPStore fallback",
            dist.get_rank(),
            e,
        )

    try:
        rank = dist.get_rank()
        world_size = dist.get_world_size()

        import os

        store = dist.TCPStore(
            host_name=os.environ["MASTER_ADDR"],
            port=int(os.environ["MASTER_PORT"]),
            world_size=world_size,
            is_master=(rank == 0),
            timeout=torch.timedelta(seconds=30),
        )

        store_key = f"rank_in_group_{group_id}"
        store.set(store_key, str(rank))

        if rank == 0:
            ranks = []
            for i in range(group_size):
                r = int(store.get(store_key).decode())
                ranks.append(r)
            ranks_tensor = torch.tensor(ranks, dtype=torch.int32)
        else:
            ranks_tensor = torch.zeros(group_size, dtype=torch.int32)

        dist.broadcast(ranks_tensor, src=0, group=group)
        ranks = ranks_tensor.tolist()

        if rank == 0:
            store.delete_key(store_key)

        GROUP_RANKS_CACHE[group_id] = ranks
        return group_rank, group_size, ranks

    except Exception as e:
        logger.warning("[rank %s] failed to get ranks via TCPStore: %s", rank, e)
        return group_rank, group_size, [dist.get_rank() for _ in range(group_size)]


class CollectiveTracer:
    """Trace collective operations for distributed training."""

    def __init__(
        self,
        trace_file=None,
        verbose=False,
        cuda_sync=False,
        mode: CommRecordMode = CommRecordMode.LITE,
        resolve_group_ranks: bool = False,
        trace_event: bool = True,
    ):
        self.trace_file = trace_file
        self.verbose = verbose
        self.cuda_sync = cuda_sync
        self.mode = mode
        self.resolve_group_ranks = resolve_group_ranks
        self.trace_event = trace_event
        self.trace_data = []
        self.original_functions = {}
        self.hooked_functions = {}
        self.has_cuda = torch.cuda.is_available()
        for func_name in function_names:
            if hasattr(dist, func_name):
                self.hooked_functions[func_name] = getattr(dist, func_name)
            else:
                logger.debug(
                    "function %s not found in torch.distributed, skipped", func_name
                )

        if not self.hooked_functions:
            logger.warning("no torch.distributed functions found to trace")

        self.call_counts = {fn: 0 for fn in self.hooked_functions}
        self.my_rank = 0
        self.my_size = 1
        self.participate_ranks = []
        self.global_rank = 0
        self._logged_active = False

    def _should_trace(self) -> bool:
        """Skip overhead on single-rank jobs unless dist reports world_size > 1."""
        if dist.is_initialized():
            return dist.get_world_size() > 1
        raw = os.environ.get("WORLD_SIZE", "1").strip()
        try:
            return int(raw) > 1
        except ValueError:
            return False

    def _maybe_sync(self) -> None:
        if self.cuda_sync and self.has_cuda:
            _cuda_sync()

    def _log(self, message):
        if not self._logged_active and self._should_trace():
            self._logged_active = True
            logger.debug("CollectiveTracer active (distributed job)")
        if self.verbose:
            logger.info("%s", message)
        if self.trace_file:
            ranked_filename = f"{self.trace_file}-{self.global_rank}"
            with open(ranked_filename, "a") as f:
                f.write(message + "\n")

    def create_trace_entry(self, func_name, start_time, duration, tensor_info):
        return {
            "function": func_name,
            "timestamp": start_time,
            "duration": duration,
            "tensor_shape": tensor_info["shape"],
            "tensor_dtype": str(tensor_info["dtype"]),
            "tensor_size": tensor_info["size"],
        }

    def _extract_group(self, args, kwargs):
        return kwargs.get("group") or (args[2] if len(args) > 2 else None)

    def _group_info(self, group):
        """Cheap group rank/size; optional full participant list."""
        if self.resolve_group_ranks:
            return get_participating_ranks(group)
        if dist.is_initialized():
            return dist.get_rank(group=group), dist.get_world_size(group=group), []
        try:
            ws = int(os.environ.get("WORLD_SIZE", "1"))
        except ValueError:
            ws = 1
        return 0, ws, []

    def _tensor_nbytes(self, args, kwargs) -> int:
        for arg in args:
            if isinstance(arg, torch.Tensor):
                return arg.element_size() * arg.numel()
        for value in kwargs.values():
            if isinstance(value, torch.Tensor):
                return value.element_size() * value.numel()
        return 0

    def _tensor_details(self, args, kwargs):
        if self.mode != CommRecordMode.FULL:
            return "", "", self._tensor_nbytes(args, kwargs)
        info = self._extract_tensor_info(args, kwargs)
        return str(info["shape"]), str(info["dtype"]), int(info["size"])

    def _finalize_collective(
        self,
        func_name,
        start_time,
        args,
        kwargs,
        *,
        cm=None,
        meta=None,
    ):
        duration_ms = (time.perf_counter() - start_time) * 1e3
        group = self._extract_group(args, kwargs)
        group_rank, group_size, participate_ranks = self._group_info(group)
        self.my_rank, self.my_size, self.participate_ranks = (
            group_rank,
            group_size,
            participate_ranks,
        )
        self.global_rank = dist.get_rank() if dist.is_initialized() else 0
        async_op = bool(kwargs.get("async_op", False))
        tensor_shape, tensor_dtype, nbytes = self._tensor_details(args, kwargs)

        if self.mode == CommRecordMode.LITE:
            record_comm_lite(
                op=func_name,
                duration_ms=duration_ms,
                group_rank=group_rank,
                group_size=group_size,
                participate_ranks=(
                    participate_ranks if self.resolve_group_ranks else None
                ),
                tensor_shape=tensor_shape,
                tensor_dtype=tensor_dtype,
                nbytes=nbytes,
                async_op=async_op,
                write_trace_event=self.trace_event,
            )
        else:
            if meta is None:
                cm, meta = begin_comm_span(
                    func_name,
                    group_rank=group_rank,
                    group_size=group_size,
                    participate_ranks=participate_ranks,
                    tensor_shape=tensor_shape,
                    tensor_dtype=tensor_dtype,
                    nbytes=nbytes,
                    async_op=async_op,
                )
            finish_comm_span(
                cm,
                meta,
                op=func_name,
                duration_ms=duration_ms,
                group_rank=group_rank,
                group_size=group_size,
            )

        if self.trace_file:
            self.trace_data.append(
                self.create_trace_entry(
                    func_name,
                    start_time,
                    duration_ms / 1e3,
                    {
                        "shape": tensor_shape or "unknown",
                        "dtype": tensor_dtype or "unknown",
                        "size": nbytes,
                    },
                )
            )
        self._log(
            f"[TRACE] rank={self.global_rank} {func_name} "
            f"duration={duration_ms:.3f}ms group_size={group_size}"
        )

    def _trace_wrapper(self, func_name, orig_func):
        class TimedWork:
            def __init__(
                self, work, start_time, tracer, func_name, args, kwargs, cm, meta
            ):
                self.work = work
                self.start_time = start_time
                self.tracer = tracer
                self.func_name = func_name
                self.args = args
                self.kwargs = kwargs
                self.cm = cm
                self.meta = meta

            def wait(self):
                result = self.work.wait()
                self.tracer._maybe_sync()
                self.tracer._finalize_collective(
                    self.func_name,
                    self.start_time,
                    self.args,
                    self.kwargs,
                    cm=self.cm,
                    meta=self.meta,
                )
                return result

            def is_completed(self):
                return self.work.is_completed()

        @wraps(orig_func)
        def wrapper(*args, **kwargs):
            if not self._should_trace():
                return orig_func(*args, **kwargs)

            self.call_counts[func_name] += 1
            self._maybe_sync()
            start_time = time.perf_counter()

            cm, meta = None, None
            if self.mode == CommRecordMode.FULL:
                group = self._extract_group(args, kwargs)
                group_rank, group_size, participate_ranks = self._group_info(group)
                tensor_shape, tensor_dtype, nbytes = self._tensor_details(args, kwargs)
                cm, meta = begin_comm_span(
                    func_name,
                    group_rank=group_rank,
                    group_size=group_size,
                    participate_ranks=participate_ranks,
                    tensor_shape=tensor_shape,
                    tensor_dtype=tensor_dtype,
                    nbytes=nbytes,
                    async_op=bool(kwargs.get("async_op", False)),
                )

            if kwargs.get("async_op", False):
                work = orig_func(*args, **kwargs)
                return TimedWork(
                    work, start_time, self, func_name, args, kwargs, cm, meta
                )

            work = orig_func(*args, **kwargs)
            self._maybe_sync()
            self._finalize_collective(
                func_name, start_time, args, kwargs, cm=cm, meta=meta
            )
            return work

        return wrapper

    def _extract_tensor_info(self, args, kwargs):
        tensor = None

        for arg in args:
            if isinstance(arg, torch.Tensor):
                tensor = arg
                break

        if tensor is None:
            for value in kwargs.values():
                if isinstance(value, torch.Tensor):
                    tensor = value
                    break

        if tensor is None and args:
            first_arg = args[0]
            for attr in dir(first_arg):
                try:
                    value = getattr(first_arg, attr)
                    if isinstance(value, torch.Tensor):
                        tensor = value
                        break
                except Exception:
                    continue

        if tensor is None:
            return {"shape": "unknown", "dtype": "unknown", "size": 0}

        return {
            "shape": tuple(tensor.shape),
            "dtype": tensor.dtype,
            "size": tensor.element_size() * tensor.numel(),
        }

    def apply_hooks(self):
        for func_name, orig_func in self.hooked_functions.items():
            if hasattr(dist, func_name):
                self.original_functions[func_name] = getattr(dist, func_name)
                setattr(dist, func_name, self._trace_wrapper(func_name, orig_func))
                self._log(f"Applyed hook to function: {func_name}")

    def remove_hooks(self):
        for func_name, orig_func in self.original_functions.items():
            if hasattr(dist, func_name):
                setattr(dist, func_name, orig_func)
                self._log(f"Removed hook from function: {func_name}")

    def get_trace_data(self):
        return self.trace_data

    def get_all_call_counts(self):
        return self.call_counts.copy()

    def export_to_csv(self, filename):
        import csv

        if not self.trace_data:
            self._log("No trace data to export.")
            return

        with open(filename, "w", newline="") as csvfile:
            fieldnames = self.trace_data[0].keys()
            writer = csv.DictWriter(csvfile, fieldnames=fieldnames)
            writer.writeheader()
            for row in self.trace_data:
                writer.writerow(row)

        self._log(f"Exported trace data to {filename}")


def _cuda_sync():
    torch.cuda.synchronize()
