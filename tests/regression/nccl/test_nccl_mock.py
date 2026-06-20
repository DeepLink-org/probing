"""NCCL mock tables: seed data and SQL visibility on macOS / dev."""

from __future__ import annotations

import os
import sys

import pytest

pytestmark = pytest.mark.skipif(
    os.environ.get("PROBING") not in ("1", "2", "followed", "nested"),
    reason="needs in-process probing engine (PROBING=1)",
)


@pytest.fixture(autouse=True)
def _enable_nccl_mock(monkeypatch):
    monkeypatch.setenv("PROBING_NCCL_MOCK", "1")


def _proxy_ops_sql_name() -> str:
    """Return SQL table name for mock proxy ops (post-``make develop`` → ``nccl.proxy_ops``)."""
    from probing import query

    for name in ("nccl.proxy_ops", "python.nccl.proxy_ops"):
        try:
            df = query(f"SELECT * FROM {name} LIMIT 1")
            if len(df.columns) > 0:
                return name
        except Exception:
            continue
    pytest.skip(
        "nccl proxy_ops not in SQL catalog — run `make develop` to rebuild probing-python"
    )


@pytest.fixture
def proxy_ops_table():
    from probing.nccl.mock import seed_mock

    seed_mock(ranks=8, ops_per_rank=3)
    return _proxy_ops_sql_name()


def test_seed_mock_writes_tables(proxy_ops_table):
    from probing.nccl.mock import PROXY_OPS_TABLE, seed_mock
    from probing import query

    counts = seed_mock(ranks=4, ops_per_rank=2)
    assert counts[PROXY_OPS_TABLE] > 0

    tables = query("SHOW TABLES")["table_name"].tolist()
    assert "proxy_ops" in tables
    assert "net_qp" in tables
    assert len(query(f"SELECT * FROM {proxy_ops_table}")) > 0


def test_mock_culprit_victim_pattern(proxy_ops_table):
    from probing.nccl.mock import _CULPRIT_RANK, _VICTIM_RANK
    from probing import query

    culprit = query(
        f"""
        SELECT sum(send_gpu_wait_ns) AS gpu_wait
        FROM {proxy_ops_table}
        WHERE rank = {_CULPRIT_RANK}
        """
    )["gpu_wait"].iloc[0]

    victim = query(
        f"""
        SELECT sum(recv_wait_ns) AS recv_wait
        FROM {proxy_ops_table}
        WHERE rank = {_VICTIM_RANK}
        """
    )["recv_wait"].iloc[0]

    assert culprit > 0
    assert victim > 0

    top_culprit = query(
        f"""
        SELECT rank, sum(send_gpu_wait_ns) AS gpu_wait
        FROM {proxy_ops_table}
        GROUP BY rank
        ORDER BY gpu_wait DESC
        LIMIT 1
        """
    )
    top_victim = query(
        f"""
        SELECT rank, sum(recv_wait_ns) AS recv_wait
        FROM {proxy_ops_table}
        GROUP BY rank
        ORDER BY recv_wait DESC
        LIMIT 1
        """
    )
    assert top_culprit["rank"].iloc[0] == _CULPRIT_RANK
    assert top_victim["rank"].iloc[0] == _VICTIM_RANK


def test_maybe_auto_seed_idempotent():
    from probing.nccl import mock as nccl_mock
    from probing import query

    table = _proxy_ops_sql_name()
    nccl_mock._seeded = False
    assert nccl_mock.maybe_auto_seed() is True
    n1 = len(query(f"SELECT * FROM {table}"))
    assert nccl_mock.maybe_auto_seed() is False
    n2 = len(query(f"SELECT * FROM {table}"))
    assert n1 == n2


@pytest.mark.skipif(sys.platform != "darwin", reason="darwin default auto-mock")
def test_darwin_auto_mock_default(monkeypatch):
    from probing.nccl import mock as nccl_mock

    monkeypatch.delenv("PROBING_NCCL_MOCK", raising=False)
    assert nccl_mock._mock_env_enabled() is True


def test_cli_seed_mock():
    from probing.nccl.__main__ import main

    assert main(["--seed-mock", "--ranks", "2", "--ops", "1"]) == 0
