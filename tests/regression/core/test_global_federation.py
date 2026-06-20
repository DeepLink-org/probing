"""Regression tests for global federated SQL queries (in-process engine)."""

from __future__ import annotations

import pytest

pytestmark = pytest.mark.integration


def test_global_cluster_nodes_explicit_select_omits_probe_tags():
    import probing

    df = probing.query("SELECT host FROM global.cluster.nodes LIMIT 5")
    assert "host" in df.columns
    assert "_host" not in df.columns
    assert "_addr" not in df.columns
    assert "_rank" not in df.columns


def test_global_cluster_nodes_select_star_includes_probe_tags():
    import probing

    df = probing.query("SELECT * FROM global.cluster.nodes LIMIT 5")
    assert "host" in df.columns
    assert "_host" in df.columns
    assert "_addr" in df.columns
    assert "_rank" in df.columns


def test_probe_cluster_nodes_omits_probe_tags():
    import probing

    df = probing.query("SELECT host FROM probe.cluster.nodes LIMIT 5")
    assert "host" in df.columns
    assert "_host" not in df.columns
    assert "_addr" not in df.columns
    assert "_rank" not in df.columns
