"""Mock ``nccl.*`` memtables for local debugging (macOS / dev without NCCL)."""

from __future__ import annotations

import os
import sys
import time
from typing import Iterable

PROXY_OPS_TABLE = "nccl.proxy_ops"
NET_QP_TABLE = "nccl.net_qp"

PROXY_OPS_COLUMNS = [
    "ts",
    "rank",
    "tp_rank",
    "pp_rank",
    "dp_rank",
    "comm_hash",
    "coll_func",
    "seq",
    "channel_id",
    "peer",
    "is_send",
    "n_steps",
    "trans_bytes",
    "send_gpu_wait_ns",
    "send_wait_ns",
    "recv_wait_ns",
    "recv_flush_wait_ns",
]

NET_QP_COLUMNS = [
    "ts",
    "rank",
    "device",
    "qp_num",
    "wr_id",
    "opcode",
    "length",
    "duration_ns",
]

# rank 2: culprit (slow GPU → high send_gpu_wait)
# rank 5: victim (waits on peers → high recv_wait)
_CULPRIT_RANK = 2
_VICTIM_RANK = 5

_seeded = False


def _mock_env_enabled() -> bool:
    default = "auto" if sys.platform == "darwin" else "0"
    raw = os.environ.get("PROBING_NCCL_MOCK", default).strip().lower()
    if raw in ("0", "off", "false", "no"):
        return False
    if raw in ("1", "true", "yes", "on"):
        return True
    if raw == "auto":
        if sys.platform == "darwin":
            return True
        if sys.platform == "linux":
            try:
                from probing.nccl import plugin_path

                plugin_path()
                return False
            except (OSError, FileNotFoundError):
                return True
    return False


def _proxy_row(
    *,
    ts_ns: int,
    rank: int,
    seq: int,
    channel_id: int,
    is_send: int,
    coll_func: str = "AllReduce",
    comm_hash: int = 0xDEAD_BEEF,
    peer: int | None = None,
    n_steps: int = 4,
    trans_bytes: int = 1 << 20,
    send_gpu_wait_ns: int = 0,
    send_wait_ns: int = 0,
    recv_wait_ns: int = 0,
    recv_flush_wait_ns: int = 0,
) -> list[object]:
    # Simple role mapping for mock fault injection (culprit=2, victim=5).
    tp = rank % 2
    pp = (rank // 2) % 2
    dp = rank // 4
    return [
        ts_ns,
        rank,
        tp,
        pp,
        dp,
        comm_hash,
        coll_func,
        seq,
        channel_id,
        peer if peer is not None else (rank + 1) % 8,
        is_send,
        n_steps,
        trans_bytes,
        send_gpu_wait_ns,
        send_wait_ns,
        recv_wait_ns,
        recv_flush_wait_ns,
    ]


def _net_qp_row(
    *,
    ts_ns: int,
    rank: int,
    wr_id: int,
    qp_num: int = 42,
    device: int = 0,
    opcode: int = 0,
    length: int = 65536,
    duration_ns: int = 1200,
) -> list[object]:
    return [ts_ns, rank, device, qp_num, wr_id, opcode, length, duration_ns]


def _iter_proxy_rows(
    ranks: int,
    ops_per_rank: int,
    base_ts_ns: int,
) -> Iterable[list[object]]:
    for seq in range(ops_per_rank):
        ts = base_ts_ns + seq * 10_000_000
        for rank in range(ranks):
            if rank == _CULPRIT_RANK:
                yield _proxy_row(
                    ts_ns=ts,
                    rank=rank,
                    seq=seq,
                    channel_id=0,
                    is_send=1,
                    send_gpu_wait_ns=8_000_000,
                    send_wait_ns=500_000,
                    recv_wait_ns=200_000,
                )
                yield _proxy_row(
                    ts_ns=ts + 1000,
                    rank=rank,
                    seq=seq,
                    channel_id=1,
                    is_send=0,
                    recv_wait_ns=300_000,
                )
            elif rank == _VICTIM_RANK:
                yield _proxy_row(
                    ts_ns=ts,
                    rank=rank,
                    seq=seq,
                    channel_id=0,
                    is_send=1,
                    send_gpu_wait_ns=100_000,
                    recv_wait_ns=150_000,
                )
                yield _proxy_row(
                    ts_ns=ts + 1000,
                    rank=rank,
                    seq=seq,
                    channel_id=0,
                    is_send=0,
                    recv_wait_ns=12_000_000,
                    recv_flush_wait_ns=800_000,
                )
            else:
                yield _proxy_row(
                    ts_ns=ts,
                    rank=rank,
                    seq=seq,
                    channel_id=0,
                    is_send=1,
                    send_gpu_wait_ns=200_000,
                    send_wait_ns=300_000,
                    recv_wait_ns=250_000,
                )


def _iter_net_qp_rows(
    ranks: int, ops_per_rank: int, base_ts_ns: int
) -> Iterable[list[object]]:
    wr = 0
    for seq in range(ops_per_rank):
        ts = base_ts_ns + seq * 10_000_000
        for rank in range(ranks):
            duration = 15_000_000 if rank == _VICTIM_RANK else 800_000
            yield _net_qp_row(
                ts_ns=ts,
                rank=rank,
                wr_id=wr,
                duration_ns=duration,
            )
            wr += 1


def seed_mock(*, ranks: int = 8, ops_per_rank: int = 5) -> dict[str, int]:
    """Write synthetic rows into ``nccl.proxy_ops`` and ``nccl.net_qp``.

    Returns row counts per table. Safe to call multiple times (appends more rows).
    """
    from probing.external_table import ExternalTable

    base_ts_ns = time.time_ns()

    proxy = ExternalTable.get_or_create(PROXY_OPS_TABLE, PROXY_OPS_COLUMNS)
    proxy_rows = list(_iter_proxy_rows(ranks, ops_per_rank, base_ts_ns))
    proxy.append_many(proxy_rows)

    net = ExternalTable.get_or_create(NET_QP_TABLE, NET_QP_COLUMNS)
    net_rows = list(_iter_net_qp_rows(ranks, ops_per_rank, base_ts_ns))
    net.append_many(net_rows)

    return {PROXY_OPS_TABLE: len(proxy_rows), NET_QP_TABLE: len(net_rows)}


def maybe_auto_seed() -> bool:
    """Seed mock tables once when ``PROBING_NCCL_MOCK`` allows it."""
    global _seeded
    if _seeded or not _mock_env_enabled():
        return False
    seed_mock()
    _seeded = True
    return True
