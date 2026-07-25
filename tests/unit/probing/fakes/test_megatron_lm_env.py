"""Unit tests for Megatron-LM path / version resolution (no real training)."""

from __future__ import annotations

from pathlib import Path


def _fake_checkout(tmp_path: Path, *, major: int, minor: int, patch: int = 0) -> Path:
    root = tmp_path / f"Megatron-LM-{major}.{minor}.{patch}"
    (root / "megatron" / "core").mkdir(parents=True)
    (root / "pretrain_gpt.py").write_text("# stub\n", encoding="utf-8")
    (root / "megatron" / "core" / "package_info.py").write_text(
        f"MAJOR = {major}\nMINOR = {minor}\nPATCH = {patch}\nPRE_RELEASE = ''\n",
        encoding="utf-8",
    )
    return root


def test_sibling_default_under_probing_parent():
    from probing.fakes.megatron_lm import probing_repo_root, sibling_megatron_lm_root

    repo = probing_repo_root()
    assert (repo / "python" / "probing" / "fakes").is_dir()
    assert sibling_megatron_lm_root() == (repo.parent / "Megatron-LM").resolve()


def test_resolve_priority_explicit_over_env(tmp_path, monkeypatch):
    from probing.fakes.megatron_lm import resolve_megatron_lm_root

    a = _fake_checkout(tmp_path, major=0, minor=18)
    b = _fake_checkout(tmp_path, major=0, minor=19)
    monkeypatch.setenv("MEGATRON_LM", str(a))

    via_env = resolve_megatron_lm_root()
    assert via_env.root == a.resolve()
    assert via_env.source == "env"
    assert via_env.ready is True
    assert via_env.version == (0, 18, 0)

    via_explicit = resolve_megatron_lm_root(b)
    assert via_explicit.root == b.resolve()
    assert via_explicit.source == "explicit"
    assert via_explicit.version == (0, 19, 0)


def test_resolve_switches_between_two_versions(tmp_path, monkeypatch):
    """Multi-version = point MEGATRON_LM at another tree; no in-process matrix."""
    from probing.fakes.megatron_lm import resolve_megatron_lm_root

    v18 = _fake_checkout(tmp_path, major=0, minor=18)
    v19 = _fake_checkout(tmp_path, major=0, minor=19)

    monkeypatch.setenv("MEGATRON_LM", str(v18))
    assert resolve_megatron_lm_root().version_text == "0.18.0"

    monkeypatch.setenv("MEGATRON_LM", str(v19))
    assert resolve_megatron_lm_root().version_text == "0.19.0"


def test_missing_checkout_not_ready(tmp_path, monkeypatch):
    from probing.fakes.megatron_lm import resolve_megatron_lm_root

    missing = tmp_path / "nope"
    monkeypatch.setenv("MEGATRON_LM", str(missing))
    co = resolve_megatron_lm_root()
    assert co.ready is False
    assert "missing" in co.reason


def test_incomplete_tree_not_ready(tmp_path, monkeypatch):
    from probing.fakes.megatron_lm import resolve_megatron_lm_root

    root = tmp_path / "half"
    root.mkdir()
    (root / "pretrain_gpt.py").write_text("#\n", encoding="utf-8")
    monkeypatch.setenv("MEGATRON_LM", str(root))
    co = resolve_megatron_lm_root()
    assert co.ready is False
    assert "not a Megatron-LM tree" in co.reason


def test_smoke_version_gate(tmp_path, monkeypatch):
    from probing.fakes.megatron_lm import (
        resolve_megatron_lm_root,
        smoke_version_allowed,
        version_in_smoke_range,
    )

    assert version_in_smoke_range((0, 12, 1)) is True
    assert version_in_smoke_range((0, 12, 0)) is False
    assert version_in_smoke_range((0, 19, 0)) is True
    assert version_in_smoke_range((0, 11, 9)) is False
    assert version_in_smoke_range((0, 21, 0)) is False

    old = _fake_checkout(tmp_path, major=0, minor=12, patch=0)
    monkeypatch.setenv("MEGATRON_LM", str(old))
    monkeypatch.delenv("MEGATRON_LM_ALLOW_ANY_VERSION", raising=False)
    co = resolve_megatron_lm_root()
    ok, why = smoke_version_allowed(co)
    assert ok is False
    assert "outside smoke range" in why

    monkeypatch.setenv("MEGATRON_LM_ALLOW_ANY_VERSION", "1")
    ok2, why2 = smoke_version_allowed(co)
    assert ok2 is True
    assert why2 == "MEGATRON_LM_ALLOW_ANY_VERSION"


def test_read_version_without_importing_megatron(tmp_path, monkeypatch):
    import sys

    from probing.fakes.megatron_lm import read_megatron_core_version

    root = _fake_checkout(tmp_path, major=0, minor=19, patch=1)
    before = {k for k in sys.modules if k == "megatron" or k.startswith("megatron.")}
    ver, text = read_megatron_core_version(root)
    after = {k for k in sys.modules if k == "megatron" or k.startswith("megatron.")}
    assert ver == (0, 19, 1)
    assert text == "0.19.1"
    assert after == before
