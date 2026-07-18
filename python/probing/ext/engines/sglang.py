"""SGLang / sglang_router metrics adapter.

Slime registers engines with ``sglang_router`` and exposes aggregated engine
Prometheus metrics on the router main port::

    GET http://{router_ip}:{router_port}/engine_metrics

See ``slime.ray.rollout.RolloutManager.get_metrics_router_addr``.
"""

from __future__ import annotations

import urllib.error
import urllib.request
from dataclasses import dataclass
from typing import Any

from probing.ext.engines.prometheus import (
    PrometheusSample,
    metric_suffix,
    parse_prometheus_text,
    pick_histogram_avg_seconds,
    pick_metric,
)

# Slime ``sglang_router`` / model gateway.
DEFAULT_METRICS_PATH = "/engine_metrics"
# ``sglang.launch_server`` with ``--enable-metrics``.
LAUNCH_SERVER_METRICS_PATH = "/metrics"


def resolve_metrics_path(metrics_path: str | None = None) -> str:
    """Resolve the Prometheus scrape path for an inference engine.

    Priority: explicit argument > ``PROBING_ENGINE_METRICS_PATH`` env > default.
    """

    import os

    raw = (metrics_path or os.environ.get("PROBING_ENGINE_METRICS_PATH", "")).strip()
    if not raw:
        return DEFAULT_METRICS_PATH
    return raw if raw.startswith("/") else f"/{raw}"

# Canonical metric names consumed by the Web UI.
NORMALIZED_METRICS = (
    "inflight_requests",
    "queue_depth",
    "throughput_tps",
    "tpot_ms",
    "ttft_ms",
    "kv_cache_usage_ratio",
    "cache_hit_ratio",
)


@dataclass(frozen=True)
class SGLangMetricsSnapshot:
    engine_id: str
    metrics_url: str
    raw_samples: tuple[PrometheusSample, ...]
    normalized: dict[str, float]
    scrape_error: str | None = None

    def to_dict(self) -> dict[str, Any]:
        return {
            "engine_id": self.engine_id,
            "metrics_url": self.metrics_url,
            "normalized": self.normalized,
            "raw_count": len(self.raw_samples),
            "scrape_error": self.scrape_error,
        }


def _latency_seconds_to_ms(value_seconds: float) -> float:
    """Convert a latency gauge from seconds to ms (legacy gauges may already be ms)."""

    return value_seconds * 1000.0 if value_seconds < 10 else value_seconds


def build_metrics_url(router_addr: str, metrics_path: str = DEFAULT_METRICS_PATH) -> str:
    base = router_addr.rstrip("/")
    if not base.startswith(("http://", "https://")):
        base = f"http://{base}"
    if not metrics_path.startswith("/"):
        metrics_path = f"/{metrics_path}"
    return f"{base}{metrics_path}"


def normalize_sglang_samples(samples: list[PrometheusSample]) -> dict[str, float]:
    """Map SGLang Prometheus samples to framework-neutral metric names."""

    normalized: dict[str, float] = {}

    inflight = pick_metric(
        samples,
        "num_requests_running",
        "num_running_reqs",
        "running_requests",
    )
    if inflight is not None:
        normalized["inflight_requests"] = inflight

    queue = pick_metric(
        samples,
        "num_queue_reqs",
        "num_waiting_reqs",
        "queue_requests",
    )
    if queue is not None:
        normalized["queue_depth"] = queue

    throughput = pick_metric(samples, "gen_throughput", "generation_throughput", "token_throughput")
    if throughput is not None:
        normalized["throughput_tps"] = throughput

    tpot_seconds = pick_metric(
        samples,
        "time_per_output_token",
        "tpot",
        "avg_time_per_output_token",
    )
    if tpot_seconds is None:
        tpot_seconds = pick_histogram_avg_seconds(
            samples,
            "inter_token_latency_seconds",
            "time_per_output_token_seconds",
        )
    if tpot_seconds is not None:
        normalized["tpot_ms"] = _latency_seconds_to_ms(tpot_seconds)

    ttft_seconds = pick_metric(
        samples,
        "time_to_first_token",
        "ttft",
        "avg_time_to_first_token",
    )
    if ttft_seconds is None:
        ttft_seconds = pick_histogram_avg_seconds(
            samples,
            "time_to_first_token_seconds",
        )
    if ttft_seconds is not None:
        normalized["ttft_ms"] = _latency_seconds_to_ms(ttft_seconds)

    cache_hit = pick_metric(samples, "cache_hit_rate", "prefix_cache_hit_rate")
    if cache_hit is not None:
        normalized["cache_hit_ratio"] = cache_hit

    kv_usage = _resolve_kv_cache_usage_ratio(samples)
    if kv_usage is not None:
        normalized["kv_cache_usage_ratio"] = kv_usage

    return normalized


def _resolve_kv_cache_usage_ratio(samples: list[PrometheusSample]) -> float | None:
    """Map SGLang KV pool occupancy to a 0..1 ratio."""

    max_tokens = pick_metric(samples, "max_total_num_tokens", "max_num_tokens", "token_capacity")
    if max_tokens in (None, 0):
        return pick_metric(samples, "token_usage_ratio", "kv_cache_usage", "cache_usage")

    # ``launch_server`` exposes pool occupancy via kv_* gauges; ``token_usage`` may stay 0.
    kv_used = pick_metric(samples, "kv_used_tokens")
    kv_evictable = pick_metric(samples, "kv_evictable_tokens")
    if kv_used is not None or kv_evictable is not None:
        occupied = (kv_used or 0.0) + (kv_evictable or 0.0)
        return min(occupied / max_tokens, 1.0)

    kv_available = pick_metric(samples, "kv_available_tokens")
    if kv_available is not None:
        occupied = max_tokens - kv_available
        if occupied >= 0:
            return min(occupied / max_tokens, 1.0)

    token_usage = pick_metric(samples, "token_usage", "num_used_tokens", "used_tokens")
    if token_usage is not None:
        return min(token_usage / max_tokens, 1.0)

    return pick_metric(samples, "token_usage_ratio", "kv_cache_usage", "cache_usage")


def fetch_sglang_metrics(
    engine_id: str,
    router_addr: str,
    *,
    metrics_path: str = DEFAULT_METRICS_PATH,
    timeout_seconds: float = 3.0,
) -> SGLangMetricsSnapshot:
    metrics_url = build_metrics_url(router_addr, metrics_path)
    request = urllib.request.Request(metrics_url, headers={"Accept": "text/plain"})
    try:
        with urllib.request.urlopen(request, timeout=timeout_seconds) as response:
            body = response.read().decode("utf-8", errors="replace")
    except urllib.error.HTTPError as exc:
        return SGLangMetricsSnapshot(
            engine_id=engine_id,
            metrics_url=metrics_url,
            raw_samples=tuple(),
            normalized={},
            scrape_error=f"HTTP {exc.code}: {exc.reason}",
        )
    except Exception as exc:  # pragma: no cover - network errors vary
        return SGLangMetricsSnapshot(
            engine_id=engine_id,
            metrics_url=metrics_url,
            raw_samples=tuple(),
            normalized={},
            scrape_error=str(exc),
        )

    samples = parse_prometheus_text(body)
    return SGLangMetricsSnapshot(
        engine_id=engine_id,
        metrics_url=metrics_url,
        raw_samples=tuple(samples),
        normalized=normalize_sglang_samples(samples),
    )


def flatten_samples_for_storage(
    engine_id: str,
    engine_type: str,
    timestamp_ns: int,
    samples: list[PrometheusSample],
) -> list[tuple[int, str, str, str, float, str]]:
    """Rows for ``InferenceEngineMetric`` persistence."""

    rows: list[tuple[int, str, str, str, float, str]] = []
    for sample in samples:
        labels = ",".join(f"{key}={value}" for key, value in sorted(sample.labels.items()))
        rows.append(
            (
                timestamp_ns,
                engine_id,
                engine_type,
                metric_suffix(sample.name),
                sample.value,
                labels,
            )
        )
    return rows
