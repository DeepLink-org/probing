"""Torch autostart profiling spec resolution."""

from __future__ import annotations

import probing
from probing.ext import torch as torch_ext
from probing.profiling.torch_probe import TorchProbeConfig


def test_torch_profiling_spec_prefers_config(monkeypatch):
    monkeypatch.delenv("PROBING_TORCH_PROFILING", raising=False)
    probing.config.set("probing.torch.profiling", "1.0,backward=on")
    try:
        assert torch_ext._torch_profiling_spec() == "1.0,backward=on"
        cfg = TorchProbeConfig.parse(torch_ext._torch_profiling_spec())
        assert cfg.enabled
        assert cfg.backward
    finally:
        probing.config.remove("probing.torch.profiling")


def test_torch_profiling_spec_falls_back_to_env(monkeypatch):
    probing.config.remove("probing.torch.profiling")
    monkeypatch.setenv("PROBING_TORCH_PROFILING", "ordered:0.1,backward=on,shadow=off")
    try:
        spec = torch_ext._torch_profiling_spec()
        assert spec == "ordered:0.1,backward=on,shadow=off"
        assert probing.config.get_str("probing.torch.profiling") == spec
        cfg = TorchProbeConfig.parse(spec)
        assert cfg.backward
        assert cfg.shadow_baseline == 0
    finally:
        probing.config.remove("probing.torch.profiling")
        monkeypatch.delenv("PROBING_TORCH_PROFILING", raising=False)
