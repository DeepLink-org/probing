"""Minimal Prometheus text exposition parser."""

from __future__ import annotations

import re
from dataclasses import dataclass
from typing import Iterable


@dataclass(frozen=True)
class PrometheusSample:
    name: str
    value: float
    labels: dict[str, str]


_METRIC_LINE = re.compile(
    r"^(?P<name>[a-zA-Z_:][a-zA-Z0-9_:]*)(?P<labels>\{[^}]*\})?\s+(?P<value>-?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][+-]?\d+)?|nan|inf|-inf)$"
)
_LABEL_PAIR = re.compile(r'([a-zA-Z_][a-zA-Z0-9_]*)="((?:\\.|[^"\\])*)"')


def _parse_labels(raw: str) -> dict[str, str]:
    if not raw:
        return {}
    labels: dict[str, str] = {}
    for match in _LABEL_PAIR.finditer(raw):
        labels[match.group(1)] = match.group(2).replace('\\"', '"').replace("\\\\", "\\")
    return labels


def parse_prometheus_text(body: str) -> list[PrometheusSample]:
    """Parse ``text/plain; version=0.0.4`` Prometheus metrics."""

    samples: list[PrometheusSample] = []
    for line in body.splitlines():
        stripped = line.strip()
        if not stripped or stripped.startswith("#"):
            continue
        match = _METRIC_LINE.match(stripped)
        if match is None:
            continue
        name = match.group("name")
        labels_raw = match.group("labels") or ""
        value_raw = match.group("value")
        if value_raw in {"nan", "inf", "-inf"}:
            continue
        samples.append(
            PrometheusSample(
                name=name,
                value=float(value_raw),
                labels=_parse_labels(labels_raw.strip("{}")),
            )
        )
    return samples


def metric_suffix(name: str) -> str:
    """Return the metric name without an optional ``namespace:`` prefix."""

    if ":" in name:
        return name.rsplit(":", 1)[-1]
    return name


def pick_metric(samples: Iterable[PrometheusSample], *candidates: str) -> float | None:
    """Return the first matching sample value for any candidate suffix."""

    wanted = {candidate.lower() for candidate in candidates}
    for sample in samples:
        suffix = metric_suffix(sample.name).lower()
        if suffix in wanted:
            return sample.value
    return None


def pick_histogram_avg_seconds(
    samples: Iterable[PrometheusSample],
    *base_names: str,
) -> float | None:
    """Return average latency in seconds from Prometheus histogram sum/count pairs."""

    wanted_bases = {name.lower() for name in base_names}
    sums: dict[tuple[str, tuple[tuple[str, str], ...]], float] = {}
    counts: dict[tuple[str, tuple[tuple[str, str], ...]], float] = {}

    for sample in samples:
        suffix = metric_suffix(sample.name).lower()
        label_key = tuple(sorted(sample.labels.items()))
        if suffix.endswith("_sum"):
            base = suffix[: -len("_sum")]
            if base in wanted_bases:
                sums[(base, label_key)] = sample.value
        elif suffix.endswith("_count"):
            base = suffix[: -len("_count")]
            if base in wanted_bases:
                counts[(base, label_key)] = sample.value

    for key, total in sums.items():
        count = counts.get(key)
        if count is not None and count > 0:
            return total / count

    for base in wanted_bases:
        total_sum = sum(value for (metric_base, _), value in sums.items() if metric_base == base)
        total_count = sum(value for (metric_base, _), value in counts.items() if metric_base == base)
        if total_count > 0:
            return total_sum / total_count

    return None
