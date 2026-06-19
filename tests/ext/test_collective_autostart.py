import pytest

from probing.profiling.collective.config import (
    collective_tracing_enabled,
    is_distributed_torch_job,
)


def test_single_process_job_disabled_by_default(monkeypatch):
    monkeypatch.setenv("WORLD_SIZE", "1")
    assert is_distributed_torch_job() is False
    assert collective_tracing_enabled() is False


def test_torchrun_auto_enables(monkeypatch):
    monkeypatch.setenv("WORLD_SIZE", "8")
    assert is_distributed_torch_job() is True
    assert collective_tracing_enabled() is True


def test_explicit_off_overrides_torchrun(monkeypatch):
    import probing

    monkeypatch.setenv("WORLD_SIZE", "8")
    probing.config.set("probing.torch.collective.enable", "off")
    assert collective_tracing_enabled() is False


def test_explicit_on_for_single_process(monkeypatch):
    import probing

    monkeypatch.setenv("WORLD_SIZE", "1")
    probing.config.set("probing.torch.collective.enable", "on")
    assert collective_tracing_enabled() is True


def test_default_mode_is_lite():
    from probing.profiling.collective.config import collective_trace_config
    from probing.profiling.collective.record import CommRecordMode

    cfg = collective_trace_config()
    assert cfg.mode == CommRecordMode.LITE


def test_full_mode_from_config():
    import probing

    from probing.profiling.collective.config import _parse_mode
    from probing.profiling.collective.record import CommRecordMode

    probing.config.set("probing.torch.collective.mode", "full")
    assert (
        _parse_mode(probing.config.get_str("probing.torch.collective.mode"))
        == CommRecordMode.FULL
    )
