#!/usr/bin/env python3
"""Long-running 64-process Megatron topology fixture for Probing UI debugging.

Torchrun starts 64 real worker processes. Before importing Probing, every
worker maps its global rank onto one of eight logical hosts with eight local
ranks. Each worker then binds an independent HTTP endpoint and reports its own
heartbeat; logical host names affect placement only, not network routing.

Sequence parallelism in Megatron uses the tensor-parallel group.  Therefore
SP=2 is represented as ``sp_rank == tp_rank`` and does not multiply world size:
``2 TP * 4 PP * 8 DP = 64 ranks``.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import contextlib
import json
import os
import threading
import time
import urllib.error
import urllib.request
from dataclasses import asdict, dataclass
from typing import Any

WORLD_SIZE = 64
HOSTS = 8
RANKS_PER_HOST = 8
TP_SIZE = 2
PP_SIZE = 4
DP_SIZE = 8
SP_SIZE = 2


class WaitCounterRecorder:
    """Record elapsed fixture waits using PyTorch wait-counter payload semantics."""

    def __init__(self) -> None:
        self._lock = threading.Lock()
        self._counters: dict[str, dict[str, int]] = {}

    @contextlib.contextmanager
    def wait(self, name: str):
        started_ns = time.perf_counter_ns()
        with self._lock:
            counter = self._counters.setdefault(
                name,
                {
                    "active_count": 0,
                    "total_calls": 0,
                    "total_time_us": 0,
                    "max_time_us": 0,
                },
            )
            counter["active_count"] += 1
        try:
            yield
        finally:
            elapsed_us = max((time.perf_counter_ns() - started_ns) // 1_000, 1)
            with self._lock:
                counter["active_count"] -= 1
                counter["total_calls"] += 1
                counter["total_time_us"] += elapsed_us
                counter["max_time_us"] = max(counter["max_time_us"], elapsed_us)

    def snapshot(self) -> dict[str, dict[str, int]]:
        with self._lock:
            return {name: dict(values) for name, values in self._counters.items()}


@dataclass(frozen=True)
class MockRank:
    rank: int
    host_index: int
    local_rank: int
    tp_rank: int
    pp_rank: int
    dp_rank: int
    sp_rank: int

    @property
    def host(self) -> str:
        return f"megatron-node-{self.host_index:02d}"

    @property
    def role(self) -> str:
        return (
            f"dp={self.dp_rank},pp={self.pp_rank},sp={self.sp_rank},tp={self.tp_rank}"
        )


def build_topology() -> list[MockRank]:
    topology = []
    for rank in range(WORLD_SIZE):
        tp_rank = rank % TP_SIZE
        pp_rank = (rank // TP_SIZE) % PP_SIZE
        dp_rank = rank // (TP_SIZE * PP_SIZE)
        topology.append(
            MockRank(
                rank=rank,
                host_index=rank // RANKS_PER_HOST,
                local_rank=rank % RANKS_PER_HOST,
                tp_rank=tp_rank,
                pp_rank=pp_rank,
                dp_rank=dp_rank,
                sp_rank=tp_rank,
            )
        )
    return topology


def groups_for_rank(topology: list[MockRank], rank: int) -> dict[str, list[int]]:
    focus = topology[rank]
    return {
        "tp": [
            item.rank
            for item in topology
            if item.dp_rank == focus.dp_rank and item.pp_rank == focus.pp_rank
        ],
        "pp": [
            item.rank
            for item in topology
            if item.dp_rank == focus.dp_rank and item.tp_rank == focus.tp_rank
        ],
        "dp": [
            item.rank
            for item in topology
            if item.pp_rank == focus.pp_rank and item.tp_rank == focus.tp_rank
        ],
        "sp": [
            item.rank
            for item in topology
            if item.dp_rank == focus.dp_rank and item.pp_rank == focus.pp_rank
        ],
    }


def validate_topology(topology: list[MockRank]) -> None:
    assert len(topology) == WORLD_SIZE
    assert len({item.role for item in topology}) == WORLD_SIZE
    assert len({item.host for item in topology}) == HOSTS
    assert all(
        sum(item.host_index == host for item in topology) == RANKS_PER_HOST
        for host in range(HOSTS)
    )
    assert {item.tp_rank for item in topology} == set(range(TP_SIZE))
    assert {item.pp_rank for item in topology} == set(range(PP_SIZE))
    assert {item.dp_rank for item in topology} == set(range(DP_SIZE))
    assert {item.sp_rank for item in topology} == set(range(SP_SIZE))
    for rank in range(WORLD_SIZE):
        groups = groups_for_rank(topology, rank)
        assert len(groups["tp"]) == TP_SIZE
        assert len(groups["pp"]) == PP_SIZE
        assert len(groups["dp"]) == DP_SIZE
        assert len(groups["sp"]) == SP_SIZE
        assert groups["sp"] == groups["tp"]


def topology_manifest(topology: list[MockRank]) -> dict[str, Any]:
    return {
        "world_size": WORLD_SIZE,
        "hosts": HOSTS,
        "ranks_per_host": RANKS_PER_HOST,
        "parallelism": {
            "tp": TP_SIZE,
            "pp": PP_SIZE,
            "dp": DP_SIZE,
            "sp": SP_SIZE,
            "sp_uses_tp_group": True,
        },
        "rank_0_groups": groups_for_rank(topology, 0),
        "ranks": [
            asdict(item) | {"host": item.host, "role": item.role} for item in topology
        ],
    }


def _request_json(url: str, *, method: str = "GET", body: Any = None) -> Any:
    data = None if body is None else json.dumps(body).encode("utf-8")
    request = urllib.request.Request(
        url,
        data=data,
        method=method,
        headers={"Content-Type": "application/json"},
    )
    with urllib.request.urlopen(request, timeout=3.0) as response:
        return json.loads(response.read().decode("utf-8"))


def _wait_for_server(base_url: str, timeout: float = 15.0) -> None:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            _request_json(f"{base_url}/apis/nodes?limit=1")
            return
        except (OSError, urllib.error.URLError, json.JSONDecodeError):
            time.sleep(0.1)
    raise RuntimeError(f"Probing server did not become ready at {base_url}")


def _configure_worker(topology: list[MockRank]) -> MockRank:
    rank = int(os.environ["RANK"])
    worker = topology[rank]
    if rank != 0:
        # A caller may set a fixed single-process bind address. Only rank 0 may
        # retain it; every other real worker must use the torchrun random bind.
        os.environ.pop("PROBING_SERVER_ADDR", None)
    os.environ["LOCAL_RANK"] = str(worker.local_rank)
    os.environ["LOCAL_WORLD_SIZE"] = str(RANKS_PER_HOST)
    os.environ["GROUP_RANK"] = str(worker.host_index)
    os.environ["NODE_RANK"] = str(worker.host_index)
    os.environ["GROUP_WORLD_SIZE"] = str(HOSTS)
    os.environ["ROLE_NAME"] = "trainer"
    os.environ["ROLE_RANK"] = str(rank)
    os.environ["ROLE_WORLD_SIZE"] = str(WORLD_SIZE)
    os.environ["PROBING_NODE_HOST"] = worker.host
    os.environ["PROBING_NODE_ROLE"] = worker.role
    os.environ["PROBING_TORCHRUN_CLUSTER"] = "1"
    os.environ["PROBING_NCCL_MOCK"] = "0"
    return worker


def _wait_for_live_workers(
    base_url: str,
    timeout: float,
    *,
    report_progress: bool = False,
) -> list[dict[str, Any]]:
    deadline = time.monotonic() + timeout
    latest: list[dict[str, Any]] = []
    previous_count = -1
    while time.monotonic() < deadline:
        payload = _request_json(f"{base_url}/apis/nodes?limit={WORLD_SIZE}")
        latest = list(payload.get("nodes", []))
        ranks = {node.get("rank") for node in latest}
        if report_progress and len(latest) != previous_count:
            print(f"heartbeat convergence: {len(latest)}/{WORLD_SIZE} live ranks")
            previous_count = len(latest)
        if len(latest) == WORLD_SIZE and ranks == set(range(WORLD_SIZE)):
            return latest
        time.sleep(1.0)
    observed_ranks = sorted(
        rank for node in latest if isinstance((rank := node.get("rank")), int)
    )
    raise RuntimeError(
        f"expected {WORLD_SIZE} live heartbeat ranks, observed {len(latest)}: "
        f"{observed_ranks}"
    )


def _verify_live_endpoints(nodes: list[dict[str, Any]]) -> None:
    addresses = [str(node.get("addr", "")) for node in nodes]
    if len(set(addresses)) != WORLD_SIZE or any(not addr for addr in addresses):
        raise RuntimeError("worker heartbeat endpoints are missing or not unique")

    def check(addr: str) -> None:
        payload = _request_json(f"http://{addr}/health")
        if payload.get("status") != "ok":
            raise RuntimeError(f"worker {addr} returned an invalid health payload")

    with concurrent.futures.ThreadPoolExecutor(max_workers=16) as executor:
        list(executor.map(check, addresses))


def _emit_training_step(
    probing: Any, iteration: int, waits: WaitCounterRecorder
) -> None:
    # Deterministic variation makes the trend useful without claiming a diagnosis.
    jitter = (iteration % 7) * 0.00015
    spike = 0.008 if iteration > 0 and iteration % 17 == 0 else 0.0
    with probing.span("train.step", source="megatron64.cpu_mock", iteration=iteration):
        with probing.span("forward", phase="forward", source="megatron64.cpu_mock"):
            time.sleep(0.0038 + jitter)
        with probing.span("tensor_parallel.all_gather", source="megatron64.cpu_mock"):
            with waits.wait("pytorch.wait_counter.fixture.ProcessGroupTP__all_gather"):
                time.sleep(0.0012)
        with probing.span("pipeline_parallel.recv", source="megatron64.cpu_mock"):
            with waits.wait("pytorch.wait_counter.fixture.ProcessGroupPP__recv"):
                time.sleep(0.0008)
        with probing.span("backward", phase="backward", source="megatron64.cpu_mock"):
            time.sleep(0.0058 + jitter + spike)
        with probing.span("data_parallel.reduce_scatter", source="megatron64.cpu_mock"):
            with waits.wait(
                "pytorch.wait_counter.fixture.ProcessGroupDP__reduce_scatter"
            ):
                time.sleep(0.0016)
        with probing.span("optimizer", phase="optimizer", source="megatron64.cpu_mock"):
            time.sleep(0.0010)


def _verify_runtime(
    base_url: str, probing: Any, nodes: list[dict[str, Any]]
) -> tuple[int, int, int, int, str]:
    _verify_live_endpoints(nodes)
    node_count = len(nodes)
    spans = probing.query(
        "SELECT count(*) AS n FROM python.trace_event "
        "WHERE name = 'train.step' AND record_type = 'span_start'"
    )
    span_count = int(spans.iloc[0]["n"])
    if span_count < 1:
        raise RuntimeError("synthetic train.step span was not persisted")
    runtime_debug = _request_json(f"{base_url}/apis/pythonext/pytorch/runtime-debug")
    wait_counters = runtime_debug.get("wait_counters", {})
    wait_rows = wait_counters.get("counters", [])
    if not wait_counters.get("available") or len(wait_rows) < 3:
        raise RuntimeError(
            f"wait counter snapshot is incomplete: {wait_counters.get('error')}"
        )
    tcpstore = runtime_debug.get("tcpstore", {})
    if not tcpstore.get("available"):
        raise RuntimeError(
            f"torchrun TCPStore inspection is unavailable: {tcpstore.get('error')}"
        )
    return (
        node_count,
        span_count,
        int(tcpstore.get("total_keys", 0)),
        len(wait_rows),
        str(wait_counters.get("source", "unknown")),
    )


def _parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--port", type=int, default=int(os.environ.get("PROBING_PORT", "18080"))
    )
    parser.add_argument(
        "--duration", type=float, default=0.0, help="seconds; 0 runs until Ctrl-C"
    )
    parser.add_argument(
        "--startup-timeout",
        type=float,
        default=float(os.environ.get("PROBING_MOCK_STARTUP_TIMEOUT_SEC", "180")),
        help="seconds allowed for 64 real worker heartbeats to converge",
    )
    parser.add_argument("--step-interval", type=float, default=0.5)
    parser.add_argument("--validate-only", action="store_true")
    parser.add_argument("--manifest", help="write the deterministic topology JSON")
    return parser.parse_args()


def main() -> int:
    args = _parse_args()
    topology = build_topology()
    validate_topology(topology)
    manifest = topology_manifest(topology)
    if args.manifest and (args.validate_only or os.environ.get("RANK", "0") == "0"):
        with open(args.manifest, "w", encoding="utf-8") as output:
            json.dump(manifest, output, indent=2)
            output.write("\n")

    if args.validate_only:
        print("validated: 8 nodes x 8 ranks = 64; TP2 PP4 DP8 SP2 (SP uses TP group)")
        print(f"rank 0 groups: {manifest['rank_0_groups']}")
        return 0

    worker = _configure_worker(topology)
    os.environ.setdefault("PROBING_PORT", str(args.port))
    os.environ.setdefault("PROBING_FAKES", "megatron")
    os.environ.setdefault("PROBING_FAKES_FORCE", "1")
    os.environ.setdefault("PROBING_FAKE_DEVICE", "cpu")
    os.environ.setdefault("PROBING_MEGATRON", "on")
    os.environ.setdefault("PROBING_MEGATRON_STEP_SYNC", "on")
    os.environ["TP_RANK"] = str(worker.tp_rank)
    os.environ.setdefault("TP_SIZE", str(TP_SIZE))
    os.environ["PP_RANK"] = str(worker.pp_rank)
    os.environ.setdefault("PP_SIZE", str(PP_SIZE))
    os.environ["DP_RANK"] = str(worker.dp_rank)
    os.environ.setdefault("DP_SIZE", str(DP_SIZE))
    os.environ["PROBING_ROLE_SP"] = str(worker.sp_rank)

    import probing
    from probing.profiling.runtime_debug import register_wait_counter_provider

    waits = WaitCounterRecorder()
    register_wait_counter_provider(waits.snapshot, source="megatron fixture")

    probing.step(0, micro_batches=1)
    probing.set_role(
        dp=worker.dp_rank,
        pp=worker.pp_rank,
        sp=worker.sp_rank,
        tp=worker.tp_rank,
    )

    base_url = f"http://127.0.0.1:{args.port}"
    _emit_training_step(probing, 0, waits)
    _wait_for_server(base_url, timeout=args.startup_timeout)
    nodes = _wait_for_live_workers(
        base_url,
        timeout=args.startup_timeout,
        report_progress=worker.rank == 0,
    )
    if worker.rank == 0:
        from probing.nccl.mock import seed_mock

        nccl_rows = seed_mock(ranks=WORLD_SIZE, ops_per_rank=3)
        (
            node_count,
            span_count,
            tcpstore_keys,
            wait_counter_count,
            wait_counter_source,
        ) = _verify_runtime(base_url, probing, nodes)
        print("validated: 8 logical nodes x 8 live ranks = 64; TP2 PP4 DP8 SP2")
        print(f"rank 0 groups: {manifest['rank_0_groups']}")
        print(f"UI: {base_url}/training")
        print(f"rank 0 NCCL mock rows: {nccl_rows}")
        print(
            f"self-check: {node_count} live worker endpoints, "
            f"{span_count} local train.step span, "
            f"{wait_counter_count} wait counters ({wait_counter_source}), "
            f"TCPStore available ({tcpstore_keys} keys)"
        )

    started = time.monotonic()
    iteration = 1
    try:
        while args.duration <= 0 or time.monotonic() - started < args.duration:
            _emit_training_step(probing, iteration, waits)
            iteration += 1
            time.sleep(max(args.step_interval, 0.0))
    except KeyboardInterrupt:
        pass
    if worker.rank == 0:
        print(f"stopped after {iteration} synthetic training steps per live rank")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
