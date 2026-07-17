"""Unit tests for the Slime process-role adapter.

Loads ``slime.py`` directly so this suite stays free of ``probing._core``.
"""

from __future__ import annotations

import importlib.util
import os
import sys
from pathlib import Path

import pytest

_SLIME_PATH = (
    Path(__file__).resolve().parents[4] / "python" / "probing" / "ext" / "slime.py"
)


def _load_slime():
    spec = importlib.util.spec_from_file_location("probing_ext_slime", _SLIME_PATH)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


slime = _load_slime()


@pytest.fixture(autouse=True)
def _clear_role_env(monkeypatch):
    for key in (
        "PROBING_RAY_PROCESS_ROLE",
        "PROBING_PROCESS_ROLE",
        "PROBING_NODE_IP",
        "SLIME_PROBING_ROLE",
        "SLIME_NODE_IP",
        "POD_IP",
        "RAY_NODE_IP",
    ):
        monkeypatch.delenv(key, raising=False)
    yield


def test_infer_role_from_slime_cmdline_markers():
    assert (
        slime.infer_role_from_cmdline("ray::RolloutManager.generate") == "rollout_actor"
    )
    assert (
        slime.infer_role_from_cmdline("ray::MegatronTrainRayActor.train") == "train_actor"
    )
    assert slime.infer_role_from_cmdline("ray::SGLangEngine") == "inference_engine"
    assert slime.infer_role_from_cmdline("python train_async.py") == "driver"
    assert slime.infer_role_from_cmdline("python my_job.py") is None


def test_apply_env_bridge_maps_slime_to_probing(monkeypatch):
    monkeypatch.setenv("SLIME_PROBING_ROLE", "rollout")
    monkeypatch.setenv("SLIME_NODE_IP", "10.0.0.1")
    slime.apply_env_bridge()
    assert os.environ["PROBING_RAY_PROCESS_ROLE"] == "rollout"
    assert os.environ["PROBING_NODE_IP"] == "10.0.0.1"


def test_apply_env_bridge_does_not_overwrite_explicit_probing(monkeypatch):
    monkeypatch.setenv("PROBING_RAY_PROCESS_ROLE", "driver")
    monkeypatch.setenv("SLIME_PROBING_ROLE", "rollout")
    slime.apply_env_bridge()
    assert os.environ["PROBING_RAY_PROCESS_ROLE"] == "driver"


def test_resolve_process_role_prefers_env_over_cmdline(monkeypatch):
    monkeypatch.setenv("SLIME_PROBING_ROLE", "rollout")
    assert (
        slime.resolve_process_role(cmdline="ray::MegatronTrainRayActor.train")
        == "rollout"
    )


def test_resolve_process_role_falls_back_to_cmdline():
    assert (
        slime.resolve_process_role(cmdline="ray::RolloutManager.generate")
        == "rollout_actor"
    )


def test_resolve_node_ip_after_bridge(monkeypatch):
    monkeypatch.setenv("SLIME_NODE_IP", "10.0.0.2")
    assert slime.resolve_node_ip() == "10.0.0.2"
