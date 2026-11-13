"""LMDeploy engine adapter: log-tailing metrics source.

LMDeploy (0.17) has no Prometheus endpoint and its TurboMind engines are
native processes, so neither the HTTP scraper nor in-process hooks apply.
The server wrapper does print one stats line per data-parallel rank roughly
every 10 seconds, which XTuner's launch scripts stream into the run log::

    [2026-11-11 03:49:36 DP3] Avg thr (in/out): 0.0 / 269.6 tokens/s, \
Server (succeeded/failed/routed/waiting): 868 / 0 / 3 / 0, \
Engine (running/waiting): 3 / 0, KV cache: 0.3%,

The adapter tails that file, attributes lines to per-rank engines, maps them
onto the framework-neutral ``normalized.*`` names the Inference page reads
(``throughput_tps`` / ``queue_depth`` / ``inflight_requests`` /
``kv_cache_usage_ratio`` ...) and writes ``inference_engine_metric`` rows
through the shared engines pipeline. All parsing paths are fail-open.
"""

from __future__ import annotations

import os
import re
import threading
import time
from pathlib import Path
from typing import Any

LINE_RE = re.compile(
    r"\[(?P<ts>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}) DP(?P<dp>\d+)\]\s*"
    r"Avg thr \(in/out\):\s*(?P<in_thr>[0-9.]+)\s*/\s*(?P<out_thr>[0-9.]+)\s*tokens/s,\s*"
    r"Server \(succeeded/failed/routed/waiting\):\s*"
    r"(?P<succeeded>\d+)\s*/\s*(?P<failed>\d+)\s*/\s*(?P<routed>\d+)\s*/\s*(?P<server_waiting>\d+),\s*"
    r"Engine \(running/waiting\):\s*(?P<running>\d+)\s*/\s*(?P<engine_waiting>\d+),\s*"
    r"KV cache:\s*(?P<kv_pct>[0-9.]+)%"
)
ANSI_RE = re.compile(r"\x1b\[[0-9;]*m")


def parse_lmdeploy_line(line: str) -> dict[str, Any] | None:
    """Parse one wrapper stats line; ``None`` when it is not one."""

    match = LINE_RE.search(ANSI_RE.sub("", line))
    if match is None:
        return None
    return {
        "ts": match.group("ts"),
        "dp": int(match.group("dp")),
        "in_thr": float(match.group("in_thr")),
        "out_thr": float(match.group("out_thr")),
        "succeeded": int(match.group("succeeded")),
        "failed": int(match.group("failed")),
        "routed": int(match.group("routed")),
        "server_waiting": int(match.group("server_waiting")),
        "running": int(match.group("running")),
        "engine_waiting": int(match.group("engine_waiting")),
        "kv_pct": float(match.group("kv_pct")),
    }


def normalize_lmdeploy_stats(stats: dict[str, Any]) -> dict[str, float]:
    """Map one parsed line onto the shared ``normalized.*`` metric names."""

    return {
        "throughput_tps": stats["out_thr"],
        "input_throughput_tps": stats["in_thr"],
        "inflight_requests": float(stats["running"]),
        "queue_depth": float(stats["engine_waiting"] + stats["server_waiting"]),
        "kv_cache_usage_ratio": stats["kv_pct"] / 100.0,
        "succeeded_requests_total": float(stats["succeeded"]),
        "failed_requests_total": float(stats["failed"]),
        "routed_requests_total": float(stats["routed"]),
    }


class _LogTailer:
    """Shared file tailer fanning lines out to per-rank queues.

    One file offset; every poll() reads whatever is new, distributes parsed
    lines into per-DP pending queues, and drains only the caller's rank —
    otherwise the first engine polled in a scrape cycle would consume and
    discard the lines of all the other ranks.
    """

    _MAX_PENDING_PER_RANK = 200

    def __init__(self: "_LogTailer", path: str):
        self.path = path
        self._offset = 0
        self._pending: dict[int, list[dict[str, Any]]] = {}
        self._lock = threading.Lock()
        self.reset_to_end()

    def reset_to_end(self) -> None:
        try:
            self._offset = os.path.getsize(self.path)
        except OSError:
            self._offset = 0
        self._pending.clear()

    def poll(self, dp_rank: int) -> list[dict[str, Any]]:
        with self._lock:
            try:
                with open(self.path, encoding="utf-8", errors="replace") as handle:
                    handle.seek(self._offset)
                    chunk = handle.read()
                    self._offset = handle.tell()
            except OSError:
                chunk = ""
            for line in chunk.splitlines():
                parsed = parse_lmdeploy_line(line)
                if parsed is not None:
                    queue = self._pending.setdefault(parsed["dp"], [])
                    queue.append(parsed)
                    if len(queue) > self._MAX_PENDING_PER_RANK:
                        del queue[: len(queue) - self._MAX_PENDING_PER_RANK]
            return self._pending.pop(dp_rank, [])


_tailers: dict[str, _LogTailer] = {}
_tailers_lock = threading.Lock()

# Latest per-engine normalized values. Log lines arrive every ~10s while the
# scraper cycles every ~5s; empty polls must not wipe the last known state
# from the engine card.
_last_good: dict[str, dict[str, float]] = {}


def _tailer_for(path: str) -> _LogTailer:
    key = str(Path(path).resolve())
    with _tailers_lock:
        tailer = _tailers.get(key)
        if tailer is None:
            tailer = _LogTailer(key)
            _tailers[key] = tailer
        return tailer


def discover_lmdeploy_ranks(log_file: str) -> list[int]:
    """Distinct DP ranks already present in the log (may be partial)."""

    ranks: set[int] = set()
    try:
        with open(log_file, encoding="utf-8", errors="replace") as handle:
            for line in handle:
                parsed = parse_lmdeploy_line(line)
                if parsed is not None:
                    ranks.add(parsed["dp"])
    except OSError:
        return []
    return sorted(ranks)


def register_lmdeploy_engines(
    log_file: str | None = None,
    *,
    ranks: list[int] | None = None,
    framework: str = "xtuner",
    labels: dict[str, str] | None = None,
    auto_scrape: bool = True,
) -> list[Any]:
    """Register one engine per lmdeploy data-parallel rank.

    ``log_file`` defaults to ``PROBING_LMDEPLOY_LOG``. ``ranks`` defaults to
    discovery from the log's DP tags, then to ``PROBING_LMDEPLOY_DP`` rank
    count (fallback 1). Discovery at registration only sees lines already
    written; for fresh logs pass ``ranks`` or the env var.
    """

    log_file = log_file or os.environ.get("PROBING_LMDEPLOY_LOG", "")
    if not log_file:
        return []
    if ranks is None:
        ranks = discover_lmdeploy_ranks(log_file)
        if not ranks:
            try:
                count = max(int(os.environ.get("PROBING_LMDEPLOY_DP", "1")), 1)
            except ValueError:
                count = 1
            ranks = list(range(count))

    from probing.ext.engines.registry import register_engine

    merged_labels = {"integration": "lmdeploy", "source": "wrapper-log"}
    merged_labels.update(labels or {})
    registrations = []
    for rank in ranks:
        registrations.append(
            register_engine(
                router_addr=f"log://{Path(log_file).resolve()}",
                engine_id=f"lmdeploy-dp{rank}",
                engine_type="lmdeploy",
                framework=framework,
                labels={**merged_labels, "dp": str(rank)},
                auto_scrape=auto_scrape,
            )
        )
    # Lines already in the file predate registration; skip them.
    _tailer_for(log_file).reset_to_end()
    return registrations


def _prefixed(metrics: dict[str, float]) -> dict[str, float]:
    """Registry keys as the Web UI's engine cards expect them.

    The cards look up ``normalized.<name>`` while the shared normalizers
    produce bare ``<name>``; store the prefixed form so the Inference page
    renders values (the history chart reads the prefixed names from the
    metric table, which already stores ``normalized.<name>``).
    """

    return {f"normalized.{name}": value for name, value in metrics.items()}


def scrape_lmdeploy_engine(registration: Any) -> dict[str, Any]:
    """Scrape hook for the shared scraper loop (one DP-rank engine)."""

    from probing.ext.engines.registry import update_scrape_result
    from probing.ext.engines.scraper import InferenceEngineMetric

    dp_label = registration.labels.get("dp", "0")
    log_file = registration.router_addr.removeprefix("log://")
    error: str | None = None
    rows_written = 0
    normalized: dict[str, float] = {}
    try:
        tailer = _tailer_for(log_file)
        for stats in tailer.poll(int(dp_label)):
            timestamp_ns = _line_timestamp_ns(stats["ts"])
            normalized = _prefixed(normalize_lmdeploy_stats(stats))
            _last_good[registration.engine_id] = normalized
            for metric_name, metric_value in normalized.items():
                InferenceEngineMetric(
                    timestamp_ns=timestamp_ns,
                    engine_id=registration.engine_id,
                    engine_type="lmdeploy",
                    metric_name=f"normalized.{metric_name}",
                    metric_value=metric_value,
                    labels=f"dp={dp_label}",
                ).save()
                rows_written += 1
    except Exception as exc:  # fail-open: surface the error on the engine card
        error = f"lmdeploy log scrape failed: {exc}"

    if not normalized:
        normalized = _last_good.get(registration.engine_id, {})
    update_scrape_result(registration.engine_id, normalized=normalized, scrape_error=error)
    return {
        "engine_id": registration.engine_id,
        "rows": rows_written,
        "error": error,
    }


def _line_timestamp_ns(ts_text: str) -> int:
    try:
        parsed = time.strptime(ts_text, "%Y-%m-%d %H:%M:%S")
        return int(time.mktime(parsed) * 1e9)
    except (ValueError, OverflowError):
        return time.time_ns()
