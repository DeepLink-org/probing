"""Framework-neutral RL telemetry data extension."""

from .protocol import CANONICAL_METRICS, RlFrameworkAdapter
from .tables import (
    RL_TABLES,
    RlBenchmark,
    RlMetric,
    RlNotice,
    RlRun,
    RlSample,
    RlSampler,
    drop_tables,
    init_tables,
)
from .writer import RlTelemetryWriter, notice


def init() -> None:
    """Enable the RL relational tables."""

    init_tables()


def deinit() -> None:
    """Disable and remove the RL relational tables."""

    drop_tables()


__all__ = [
    "CANONICAL_METRICS",
    "RL_TABLES",
    "RlBenchmark",
    "RlFrameworkAdapter",
    "RlMetric",
    "RlNotice",
    "RlRun",
    "RlSample",
    "RlSampler",
    "RlTelemetryWriter",
    "deinit",
    "drop_tables",
    "init",
    "init_tables",
    "notice",
]
