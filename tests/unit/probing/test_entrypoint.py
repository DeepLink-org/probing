"""Unit tests for process entrypoint detection."""

from __future__ import annotations

import sys

import pytest

from probing import _entrypoint as ep


@pytest.mark.parametrize(
    ("argv", "env", "expected"),
    [
        (["python", "-m", "torch.distributed.run", "--standalone"], {}, True),
        (["-m", "--help"], {}, True),
        (["/usr/bin/torchrun", "train.py"], {}, True),
        (["python", "examples/imagenet_with_span.py"], {"RANK": "0"}, False),
        (["python", "examples/imagenet_with_span.py"], {"LOCAL_RANK": "1"}, False),
        (["python", "-m", "torch.distributed.run"], {"TORCHELASTIC_RUN_ID": "x"}, True),
        (["python", "examples/tracing.py"], {}, False),
        (["-m", "probing.skills"], {}, False),
    ],
)
def test_is_elastic_supervisor(argv, env, expected, monkeypatch):
    monkeypatch.setattr(sys, "argv", argv)
    for key in ("RANK", "LOCAL_RANK", "TORCHELASTIC_RUN_ID"):
        monkeypatch.delenv(key, raising=False)
    for key, value in env.items():
        monkeypatch.setenv(key, value)
    assert ep.is_elastic_supervisor() is expected
