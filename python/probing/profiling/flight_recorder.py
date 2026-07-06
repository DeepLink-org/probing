"""PyTorch NCCL Flight Recorder bridge.

This module reads PyTorch's in-process Flight Recorder buffer and persists a
flattened view into probing mmap tables. PyTorch exposes the data through an
unstable/private API, so all calls are feature-detected and best-effort.
"""

from __future__ import annotations

import json
import os
import pickle
import time
from dataclasses import dataclass
from typing import Any, Iterable

from probing.core import table


@table("torch_nccl_flight_record")
@dataclass
class TorchNcclFlightRecord:
    """PyTorch Flight Recorder collective entries."""

    snapshot_id: int = 0
    rank: int = -1
    world_size: int = -1
    record_id: int = -1
    pg_id: str = ""
    pg_name: str = ""
    pg_desc: str = ""
    collective_seq_id: int = -1
    p2p_seq_id: int = -1
    op_id: int = -1
    profiling_name: str = ""
    state: str = ""
    input_sizes: str = ""
    input_dtypes: str = ""
    output_sizes: str = ""
    output_dtypes: str = ""
    time_created_ns: int = -1
    time_discovered_started_ns: int = -1
    time_discovered_completed_ns: int = -1
    duration_ns: int = -1
    timeout_ms: int = -1
    retired: int = 0
    is_p2p: int = 0
    frames: str = ""
    top_frame: str = ""


@table("torch_nccl_pg_status")
@dataclass
class TorchNcclPgStatus:
    """PyTorch Flight Recorder process-group status."""

    snapshot_id: int = 0
    rank: int = -1
    world_size: int = -1
    pg_id: str = ""
    pg_name: str = ""
    pg_desc: str = ""
    ranks: str = ""
    last_enqueued_collective: int = -1
    last_started_collective: int = -1
    last_completed_collective: int = -1


def _json(value: Any) -> str:
    if value is None:
        return ""
    try:
        return json.dumps(value, ensure_ascii=False, sort_keys=True)
    except Exception:
        return str(value)


def _int(value: Any, default: int = -1) -> int:
    if value is None:
        return default
    try:
        return int(value)
    except (TypeError, ValueError):
        return default


def _bool_int(value: Any) -> int:
    return 1 if bool(value) else 0


def _rank_world() -> tuple[int, int]:
    try:
        import torch.distributed as dist

        if dist.is_available() and dist.is_initialized():
            return int(dist.get_rank()), int(dist.get_world_size())
    except Exception:
        pass

    def env_int(name: str, default: int) -> int:
        try:
            return int(os.environ.get(name, str(default)))
        except ValueError:
            return default

    return env_int("RANK", -1), env_int("WORLD_SIZE", -1)


def _pg_parts(value: Any) -> tuple[str, str, str]:
    if isinstance(value, (tuple, list)):
        pg_name = str(value[0]) if len(value) > 0 else ""
        pg_desc = str(value[1]) if len(value) > 1 else ""
        return pg_name, pg_desc, _json(value)
    if value is None:
        return "", "", ""
    text = str(value)
    return text, "", text


def _top_frame(frames: Iterable[Any]) -> str:
    for frame in frames:
        if not isinstance(frame, dict):
            return str(frame)
        name = str(frame.get("name") or "")
        filename = str(frame.get("filename") or "")
        line = _int(frame.get("line"), 0)
        if filename or name:
            return f"{name} ({filename}:{line})"
    return ""


def _dump_nccl_trace(
    *,
    include_collectives: bool = True,
    include_stack_traces: bool = True,
    only_active: bool = False,
) -> dict[str, Any]:
    import torch

    c10d = getattr(getattr(torch, "_C", None), "_distributed_c10d", None)
    dump = getattr(c10d, "_dump_nccl_trace", None)
    if dump is None:
        raise RuntimeError(
            "PyTorch Flight Recorder API is unavailable "
            "(requires torch._C._distributed_c10d._dump_nccl_trace, PyTorch >= 2.5)"
        )

    try:
        payload = dump(
            includeCollectives=include_collectives,
            includeStackTraces=include_stack_traces,
            onlyActive=only_active,
        )
    except TypeError:
        payload = dump()

    data = pickle.loads(payload)
    if not isinstance(data, dict):
        raise RuntimeError(f"unexpected Flight Recorder payload type: {type(data)!r}")
    return data


def snapshot(
    *,
    include_stack_traces: bool = True,
    only_active: bool = False,
    persist: bool = True,
) -> dict[str, Any]:
    """Read the local rank Flight Recorder buffer and optionally persist rows."""

    snapshot_id = time.time_ns()
    rank, world_size = _rank_world()
    data = _dump_nccl_trace(
        include_collectives=True,
        include_stack_traces=include_stack_traces,
        only_active=only_active,
    )

    records: list[TorchNcclFlightRecord] = []
    for entry in data.get("entries") or []:
        if not isinstance(entry, dict):
            continue
        frames = entry.get("frames") or []
        pg_name, pg_desc, pg_repr = _pg_parts(entry.get("process_group"))
        records.append(
            TorchNcclFlightRecord(
                snapshot_id=snapshot_id,
                rank=rank,
                world_size=world_size,
                record_id=_int(entry.get("record_id")),
                pg_id=str(entry.get("pg_id") or pg_name or pg_repr),
                pg_name=pg_name,
                pg_desc=pg_desc,
                collective_seq_id=_int(entry.get("collective_seq_id")),
                p2p_seq_id=_int(entry.get("p2p_seq_id")),
                op_id=_int(entry.get("op_id")),
                profiling_name=str(entry.get("profiling_name") or ""),
                state=str(entry.get("state") or ""),
                input_sizes=_json(entry.get("input_sizes")),
                input_dtypes=_json(entry.get("input_dtypes")),
                output_sizes=_json(entry.get("output_sizes")),
                output_dtypes=_json(entry.get("output_dtypes")),
                time_created_ns=_int(entry.get("time_created_ns")),
                time_discovered_started_ns=_int(
                    entry.get("time_discovered_started_ns")
                ),
                time_discovered_completed_ns=_int(
                    entry.get("time_discovered_completed_ns")
                ),
                duration_ns=_int(entry.get("duration_ns")),
                timeout_ms=_int(entry.get("timeout_ms")),
                retired=_bool_int(entry.get("retired")),
                is_p2p=_bool_int(entry.get("is_p2p")),
                frames=_json(frames),
                top_frame=_top_frame(frames),
            )
        )

    statuses: list[TorchNcclPgStatus] = []
    pg_config = data.get("pg_config") or {}
    pg_status = data.get("pg_status") or {}
    for pg_id, status in pg_status.items():
        if not isinstance(status, dict):
            continue
        cfg = pg_config.get(pg_id) if isinstance(pg_config, dict) else None
        cfg = cfg if isinstance(cfg, dict) else {}
        statuses.append(
            TorchNcclPgStatus(
                snapshot_id=snapshot_id,
                rank=rank,
                world_size=world_size,
                pg_id=str(pg_id),
                pg_name=str(cfg.get("name") or pg_id),
                pg_desc=str(cfg.get("desc") or ""),
                ranks=str(cfg.get("ranks") or ""),
                last_enqueued_collective=_int(status.get("last_enqueued_collective")),
                last_started_collective=_int(status.get("last_started_collective")),
                last_completed_collective=_int(status.get("last_completed_collective")),
            )
        )

    if persist:
        if records:
            TorchNcclFlightRecord.append_many(records)
        if statuses:
            TorchNcclPgStatus.append_many(statuses)

    return {
        "ok": True,
        "snapshot_id": snapshot_id,
        "rank": rank,
        "world_size": world_size,
        "records": len(records),
        "process_groups": len(statuses),
        "version": str(data.get("version") or ""),
        "comm_lib_version": str(data.get("comm_lib_version") or ""),
    }


def snapshot_json(
    *,
    include_stack_traces: bool = True,
    only_active: bool = False,
    persist: bool = True,
) -> str:
    try:
        result = snapshot(
            include_stack_traces=include_stack_traces,
            only_active=only_active,
            persist=persist,
        )
    except Exception as exc:
        result = {"ok": False, "error": str(exc)}
    return json.dumps(result, ensure_ascii=False, sort_keys=True)
