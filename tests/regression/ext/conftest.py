"""Fixtures for framework extension tests under ``tests/regression/ext/``."""

from __future__ import annotations

import sys

import pytest

import probing
from probing.ext import megatron as megatron_ext
from probing.ext import vllm as vllm_ext

# Subprocess workers + mock helpers are not tests; --doctest-modules still imports
# every .py under testpaths unless ignored here.
collect_ignore = [
    "megatron_integration_worker.py",
    "vllm_integration_worker.py",
    "_megatron_contract.py",
    "_vllm_contract.py",
]


@pytest.fixture(autouse=True)
def _reset_megatron_ext_state():
    megatron_ext._PARALLEL_STATE_INIT = False
    megatron_ext._TRAINING_INIT = False
    megatron_ext._LAST_ROLE = None
    megatron_ext._LAST_ITERATION = None
    yield
    for key in list(sys.modules):
        if key == "megatron" or key.startswith("megatron."):
            del sys.modules[key]


@pytest.fixture(autouse=True)
def _reset_vllm_ext_state():
    vllm_ext._METAL_INIT = False
    vllm_ext._ENGINE_INIT_DONE.clear()
    vllm_ext._LAST_ROLE = None
    vllm_ext._LAST_STEP = None
    yield
    for key in list(sys.modules):
        if key == "vllm" or key.startswith("vllm.") or key == "vllm_metal":
            del sys.modules[key]


@pytest.fixture(autouse=True)
def _reset_probing_role():
    probing.clear_role()
    yield
    probing.clear_role()


@pytest.fixture(autouse=True)
def _reset_step_coordinates():
    """Isolate step coordinates across contract tests (Megatron + vLLM share globals)."""
    probing.step(0)
    megatron_ext._LAST_ITERATION = None
    vllm_ext._LAST_STEP = None
    yield
    probing.step(0)
    megatron_ext._LAST_ITERATION = None
    vllm_ext._LAST_STEP = None


@pytest.fixture
def megatron_env(monkeypatch):
    monkeypatch.setenv("PROBING_MEGATRON", "on")
    monkeypatch.setenv("PROBING_MEGATRON_STEP_SYNC", "on")


@pytest.fixture
def vllm_env(monkeypatch):
    monkeypatch.setenv("PROBING_VLLM", "on")
    monkeypatch.setenv("PROBING_VLLM_STEP_SYNC", "on")
