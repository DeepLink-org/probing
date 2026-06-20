"""Collective tracing policy: when hooks install and which mode defaults apply."""

import pytest

from probing.profiling.collective.config import (
    collective_trace_config,
    collective_tracing_enabled,
    is_distributed_torch_job,
)
from probing.profiling.collective.record import CommRecordMode


@pytest.mark.training_observability
class TestCollectiveAutostartPolicy:
    def test_single_process_job_disabled_by_default(self, monkeypatch):
        monkeypatch.setenv("WORLD_SIZE", "1")
        assert is_distributed_torch_job() is False
        assert collective_tracing_enabled() is False

    def test_torchrun_auto_enables(self, monkeypatch):
        monkeypatch.setenv("WORLD_SIZE", "8")
        assert is_distributed_torch_job() is True
        assert collective_tracing_enabled() is True

    def test_explicit_off_overrides_torchrun(self, monkeypatch):
        import probing

        monkeypatch.setenv("WORLD_SIZE", "8")
        probing.config.set("probing.torch.collective.enable", "off")
        assert collective_tracing_enabled() is False

    def test_explicit_on_for_single_process(self, monkeypatch):
        import probing

        monkeypatch.setenv("WORLD_SIZE", "1")
        probing.config.set("probing.torch.collective.enable", "on")
        assert collective_tracing_enabled() is True


@pytest.mark.training_observability
class TestCollectiveModeDefaults:
    def test_default_mode_is_lite(self):
        cfg = collective_trace_config()
        assert cfg.mode == CommRecordMode.LITE
        assert cfg.trace_event is True

    def test_full_mode_from_config(self):
        import probing

        probing.config.set("probing.torch.collective.mode", "full")
        cfg = collective_trace_config()
        assert cfg.mode == CommRecordMode.FULL
        assert cfg.resolve_group_ranks is True

    def test_trace_event_can_be_disabled_in_lite(self):
        import probing

        probing.config.set("probing.torch.collective.trace_event", "off")
        cfg = collective_trace_config()
        assert cfg.trace_event is False
