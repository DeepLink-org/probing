"""Mock ``nccl.*`` memtables for local debugging (macOS / dev without NCCL)."""

from __future__ import annotations

import os
import sys
import time
from typing import Iterable

PROXY_OPS_TABLE = "nccl.proxy_ops"
COLL_PERF_TABLE = "nccl.coll_perf"
INFLIGHT_OPS_TABLE = "nccl.inflight_ops"
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
    "send_peer_wait_ns",
    "send_wait_ns",
    "recv_wait_ns",
    "recv_flush_wait_ns",
]

COLL_PERF_COLUMNS = [
    "ts",
    "rank",
    "tp_rank",
    "pp_rank",
    "dp_rank",
    "comm_hash",
    "n_ranks",
    "coll_func",
    "seq",
    "is_p2p",
    "peer",
    "count",
    "msg_size_bytes",
    "dtype",
    "algo",
    "proto",
    "n_channels",
    "exec_time_ns",
    "enqueue_time_ns",
    "timing_source",
    "algobw_gbps",
]

INFLIGHT_OPS_COLUMNS = [
    "ts",
    "rank",
    "comm_hash",
    "coll_func",
    "seq",
    "kind",
    "channel_id",
    "peer",
    "is_send",
    "start_ns",
    "age_ns",
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
    send_peer_wait_ns: int = 0,
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
        send_peer_wait_ns,
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
                    send_peer_wait_ns=50_000,  # v4-only signal (v3 rows: 0)
                    send_wait_ns=300_000,
                    recv_wait_ns=250_000,
                )


def _iter_coll_perf_rows(
    ranks: int,
    ops_per_rank: int,
    base_ts_ns: int,
) -> Iterable[list[object]]:
    msg_bytes = 1 << 24  # 16 MiB fp16 AllReduce
    for seq in range(ops_per_rank):
        ts = base_ts_ns + seq * 10_000_000
        for rank in range(ranks):
            # culprit rank finishes slower → lower bandwidth
            exec_ns = 9_000_000 if rank == _CULPRIT_RANK else 3_000_000
            tp = rank % 2
            pp = (rank // 2) % 2
            dp = rank // 4
            yield [
                ts + exec_ns,
                rank,
                tp,
                pp,
                dp,
                0xDEAD_BEEF,
                ranks,  # n_ranks: communicator size (v4 init metadata)
                "AllReduce",
                seq,
                0,  # is_p2p
                -1,  # peer
                msg_bytes // 2,  # fp16 count
                msg_bytes,
                "ncclFloat16",
                "Ring",
                "Simple",
                4,
                exec_ns,
                50_000,  # enqueue_time_ns: host-side enqueue is much shorter
                "kernel_gpu",  # v4: GPU globaltimer window
                msg_bytes / exec_ns,  # bytes/ns == GB/s
            ]


def _iter_inflight_rows(base_ts_ns: int) -> Iterable[list[object]]:
    # victim rank stuck in an AllReduce for 30s (hang scenario)
    age_ns = 30_000_000_000
    yield [
        base_ts_ns,
        _VICTIM_RANK,
        0xDEAD_BEEF,
        "AllReduce",
        99,
        "coll",
        -1,
        -1,
        -1,
        base_ts_ns - age_ns,
        age_ns,
    ]


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
    """Write synthetic rows into the ``nccl.*`` mock tables.

    Returns row counts per table. Safe to call multiple times (appends more rows).
    """
    from probing.external_table import ExternalTable

    base_ts_ns = time.time_ns()

    proxy = ExternalTable.get_or_create(PROXY_OPS_TABLE, PROXY_OPS_COLUMNS)
    proxy_rows = list(_iter_proxy_rows(ranks, ops_per_rank, base_ts_ns))
    proxy.append_many(proxy_rows)

    coll = ExternalTable.get_or_create(COLL_PERF_TABLE, COLL_PERF_COLUMNS)
    coll_rows = list(_iter_coll_perf_rows(ranks, ops_per_rank, base_ts_ns))
    coll.append_many(coll_rows)

    inflight = ExternalTable.get_or_create(INFLIGHT_OPS_TABLE, INFLIGHT_OPS_COLUMNS)
    inflight_rows = list(_iter_inflight_rows(base_ts_ns))
    inflight.append_many(inflight_rows)

    net = ExternalTable.get_or_create(NET_QP_TABLE, NET_QP_COLUMNS)
    net_rows = list(_iter_net_qp_rows(ranks, ops_per_rank, base_ts_ns))
    net.append_many(net_rows)

    return {
        PROXY_OPS_TABLE: len(proxy_rows),
        COLL_PERF_TABLE: len(coll_rows),
        INFLIGHT_OPS_TABLE: len(inflight_rows),
        NET_QP_TABLE: len(net_rows),
    }


def maybe_auto_seed() -> bool:
    """Seed mock tables once when ``PROBING_NCCL_MOCK`` allows it."""
    global _seeded
    if _seeded or not _mock_env_enabled():
        return False
    seed_mock()
    _seeded = True
    return True
