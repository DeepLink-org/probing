"""Run **real** Megatron-LM ``pretrain_gpt.py`` with bottom-layer fakes only.

Philosophy: keep Megatron-LM / megatron-core source intact; only replace what
cannot run on macOS — CUDA device APIs (→ meta/cpu/mps), and missing packages
(triton / flash-attn).

Path resolution (first hit wins)::

    1. ``MEGATRON_LM`` / ``--megatron-lm``
    2. sibling ``<probing-repo>/../Megatron-LM``

**Versions:** probing pins to **one checkout at a time**. To exercise another
Megatron-Core release, point ``MEGATRON_LM`` at that tree (no multi-version
matrix in-process). Smoke defaults are validated against **≥0.12.1 and <0.21**;
see ``SMOKE_MEGATRON_CORE_MIN`` / ``SMOKE_MEGATRON_CORE_MAX_EXCLUSIVE`` and
``MEGATRON_LM_ALLOW_ANY_VERSION``.
"""

from __future__ import annotations

import os
import re
import runpy
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Optional


BOTTOM_LAYER_SPECS = (
    # Invent only packages that hard-fail imports on macOS.
    # Do NOT invent transformer_engine — Megatron falls back to local impl.
    # Do NOT invent apex — Megatron falls back to torch.optim + WrappedTorchNorm.
    "triton",
    "flash_attn",
)

# Inclusive min / exclusive max for bottom-fake smoke (major, minor, patch).
# Other versions may work; set MEGATRON_LM_ALLOW_ANY_VERSION=1 to skip the gate.
SMOKE_MEGATRON_CORE_MIN: tuple[int, int, int] = (0, 12, 1)
SMOKE_MEGATRON_CORE_MAX_EXCLUSIVE: tuple[int, int, int] = (0, 21, 0)
# Backward-compatible alias used in messages / older docs.
SMOKE_MEGATRON_CORE_RANGE: tuple[tuple[int, int, int], tuple[int, int, int]] = (
    SMOKE_MEGATRON_CORE_MIN,
    SMOKE_MEGATRON_CORE_MAX_EXCLUSIVE,
)


@dataclass(frozen=True)
class MegatronLmCheckout:
    """Resolved Megatron-LM tree used by the real-code runner."""

    root: Path
    source: str  # "env" | "explicit" | "sibling"
    pretrain_gpt: Path
    version: Optional[tuple[int, int, int]]
    version_text: Optional[str]
    ready: bool
    reason: str = ""


def probing_repo_root() -> Path:
    """``probing/`` repo root (``python/probing/fakes/`` → parents[3])."""
    return Path(__file__).resolve().parents[3]


def sibling_megatron_lm_root() -> Path:
    """Default checkout: ``../Megatron-LM`` next to the probing repo."""
    return (probing_repo_root().parent / "Megatron-LM").resolve()


def default_megatron_lm_root() -> Path:
    """Backward-compatible alias: env ``MEGATRON_LM`` or sibling path."""
    return resolve_megatron_lm_root().root


def is_megatron_lm_checkout(root: Path) -> bool:
    """True when ``root`` looks like an NVIDIA Megatron-LM tree."""
    return (root / "pretrain_gpt.py").is_file() and (root / "megatron").is_dir()


def read_megatron_core_version(
    root: Path,
) -> tuple[Optional[tuple[int, int, int]], Optional[str]]:
    """Read ``(major, minor, patch)`` from checkout ``package_info.py`` (no import).

    Does not load ``megatron`` into ``sys.modules``. Returns ``(None, None)`` if
    the file is missing or unparsable.
    """
    info = root / "megatron" / "core" / "package_info.py"
    if not info.is_file():
        return None, None
    try:
        text = info.read_text(encoding="utf-8")
    except OSError:
        return None, None

    def _int(name: str) -> Optional[int]:
        m = re.search(rf"^{name}\s*=\s*(\d+)\s*$", text, flags=re.MULTILINE)
        return int(m.group(1)) if m else None

    major, minor, patch = _int("MAJOR"), _int("MINOR"), _int("PATCH")
    if major is None or minor is None or patch is None:
        return None, None
    ver = (major, minor, patch)
    return ver, f"{major}.{minor}.{patch}"


def version_in_smoke_range(version: tuple[int, int, int]) -> bool:
    return SMOKE_MEGATRON_CORE_MIN <= version < SMOKE_MEGATRON_CORE_MAX_EXCLUSIVE


def resolve_megatron_lm_root(
    explicit: str | Path | None = None,
    *,
    env: Optional[dict[str, str]] = None,
) -> MegatronLmCheckout:
    """Resolve which Megatron-LM tree to use.

    Priority: ``explicit`` → ``MEGATRON_LM`` → sibling ``../Megatron-LM``.

    Multiple installed / checked-out versions are **not** selected automatically;
    set ``MEGATRON_LM`` (or ``explicit``) to switch. Pip ``megatron-core`` is
    ignored once the checkout is on ``sys.path`` (see :func:`bootstrap`).
    """
    environ = env if env is not None else os.environ
    if explicit is not None and str(explicit).strip():
        root = Path(str(explicit)).expanduser().resolve()
        source = "explicit"
    else:
        raw = (environ.get("MEGATRON_LM") or "").strip()
        if raw:
            root = Path(raw).expanduser().resolve()
            source = "env"
        else:
            root = sibling_megatron_lm_root()
            source = "sibling"

    script = root / "pretrain_gpt.py"
    version, version_text = read_megatron_core_version(root)
    if not root.is_dir():
        return MegatronLmCheckout(
            root=root,
            source=source,
            pretrain_gpt=script,
            version=version,
            version_text=version_text,
            ready=False,
            reason=f"directory missing: {root}",
        )
    if not is_megatron_lm_checkout(root):
        return MegatronLmCheckout(
            root=root,
            source=source,
            pretrain_gpt=script,
            version=version,
            version_text=version_text,
            ready=False,
            reason=(
                f"not a Megatron-LM tree (need pretrain_gpt.py + megatron/): {root}"
            ),
        )
    return MegatronLmCheckout(
        root=root,
        source=source,
        pretrain_gpt=script,
        version=version,
        version_text=version_text,
        ready=True,
    )


def smoke_version_allowed(
    checkout: MegatronLmCheckout,
    *,
    env: Optional[dict[str, str]] = None,
) -> tuple[bool, str]:
    """Whether bottom-fake smoke should run against this checkout's version."""
    environ = env if env is not None else os.environ
    if (environ.get("MEGATRON_LM_ALLOW_ANY_VERSION") or "").strip().lower() in {
        "1",
        "true",
        "yes",
        "on",
    }:
        return True, "MEGATRON_LM_ALLOW_ANY_VERSION"
    if checkout.version is None:
        return True, "version unknown (no package_info.py gate)"
    if version_in_smoke_range(checkout.version):
        return True, f"in smoke range {SMOKE_MEGATRON_CORE_RANGE}"
    lo, hi = SMOKE_MEGATRON_CORE_RANGE
    return (
        False,
        f"megatron-core {checkout.version_text} outside smoke range "
        f">={lo[0]}.{lo[1]}.{lo[2]}, <{hi[0]}.{hi[1]}.{hi[2]}; "
        f"set MEGATRON_LM_ALLOW_ANY_VERSION=1 to override",
    )


def _purge_megatron_modules() -> list[str]:
    """Drop already-imported megatron modules so Megatron-LM path wins."""
    removed: list[str] = []
    for name in list(sys.modules):
        if name == "megatron" or name.startswith("megatron."):
            del sys.modules[name]
            removed.append(name)
    return removed


def bootstrap(*, megatron_lm: Path, device: str | None = None) -> Path:
    checkout = resolve_megatron_lm_root(megatron_lm)
    if not checkout.ready:
        raise SystemExit(
            f"Megatron-LM not ready ({checkout.reason}). "
            "Set MEGATRON_LM=/path/to/Megatron-LM"
        )
    allowed, why = smoke_version_allowed(checkout)
    if not allowed:
        raise SystemExit(why)

    os.environ.setdefault("PROBING_FAKES", "1")
    # Do NOT set PROBING_FAKES_FORCE for megatron — we want real Megatron code.
    # Force only the CUDA-only deps via install(force=True, specs=BOTTOM...).
    if device:
        os.environ["PROBING_FAKE_DEVICE"] = device
    else:
        # Real Megatron ops need a concrete device; meta breaks collectives / autograd.
        os.environ.setdefault("PROBING_FAKE_DEVICE", "cpu")
    os.environ.setdefault("PROBING_MEGATRON", "on")
    os.environ.setdefault("PROBING_MEGATRON_STEP_SYNC", "on")
    os.environ.setdefault("PROBING_NCCL_MOCK", "1")
    # Faster failure loop while iterating on bottom fakes.
    os.environ.setdefault("PROBING_CRASH_NO_GRACE", "1")
    # Single-process rendezvous defaults (used if --fake-process-group is off).
    os.environ.setdefault("MASTER_ADDR", "127.0.0.1")
    os.environ.setdefault("MASTER_PORT", "29500")
    os.environ.setdefault("RANK", "0")
    os.environ.setdefault("WORLD_SIZE", "1")
    os.environ.setdefault("LOCAL_RANK", "0")

    root = str(checkout.root)
    # Prefer the checkout over any pip megatron-core.
    while root in sys.path:
        sys.path.remove(root)
    sys.path.insert(0, root)
    purged = _purge_megatron_modules()

    from probing.fakes import install
    from probing.fakes.bottom import apply_bottom_patches, skip_dataset_cpp_helpers
    from probing.fakes.journal import record_fake_event

    # Bottom layer only — never invent megatron.* when running real Megatron-LM.
    install(force=True, specs=BOTTOM_LAYER_SPECS, remap_device=True)
    apply_bottom_patches()
    skip_dataset_cpp_helpers()
    record_fake_event(
        "megatron_lm",
        "bootstrap",
        attrs={
            "megatron_lm": root,
            "source": checkout.source,
            "version": checkout.version_text,
            "purged_modules": len(purged),
            "mode": "real_megatron_bottom_fakes",
            "specs": list(BOTTOM_LAYER_SPECS),
            "version_gate": why,
        },
    )
    print(
        f"[probing.fakes] real Megatron-LM at {root} "
        f"(source={checkout.source}, version={checkout.version_text or '?'}; "
        f"bottom fakes: {', '.join(BOTTOM_LAYER_SPECS)}; cuda→"
        f"{os.environ.get('PROBING_FAKE_DEVICE', 'cpu')})",
        flush=True,
    )
    return checkout.pretrain_gpt


def main(argv: list[str] | None = None) -> int:
    argv = list(sys.argv[1:] if argv is None else argv)
    device = None
    megatron_explicit: str | None = None
    filtered: list[str] = []
    i = 0
    while i < len(argv):
        if argv[i] == "--device" and i + 1 < len(argv):
            device = argv[i + 1]
            i += 2
            continue
        if argv[i] == "--megatron-lm" and i + 1 < len(argv):
            megatron_explicit = argv[i + 1]
            os.environ["MEGATRON_LM"] = argv[i + 1]
            i += 2
            continue
        filtered.append(argv[i])
        i += 1

    checkout = resolve_megatron_lm_root(megatron_explicit)
    script = bootstrap(megatron_lm=checkout.root, device=device)

    # Baseline smoke flags; user argv overrides / extends them.
    defaults = [
        "--num-layers",
        "2",
        "--hidden-size",
        "64",
        "--num-attention-heads",
        "4",
        "--seq-length",
        "32",
        "--max-position-embeddings",
        "32",
        "--micro-batch-size",
        "1",
        "--global-batch-size",
        "1",
        "--train-iters",
        "2",
        "--eval-iters",
        "0",
        "--eval-interval",
        "1000",
        "--save-interval",
        "1000",
        "--log-interval",
        "1",
        "--num-workers",
        "0",
        "--mock-data",
        "--tokenizer-type",
        "NullTokenizer",
        "--vocab-size",
        "1024",
        "--no-gradient-accumulation-fusion",
        "--no-masked-softmax-fusion",
        "--no-bias-gelu-fusion",
        "--no-bias-dropout-fusion",
        "--no-persist-layer-norm",
        "--use-cpu-initialization",
        "--lr",
        "1e-4",
        "--min-lr",
        "1e-5",
        "--clip-grad",
        "0.0",
        # Skip real NCCL/gloo — Megatron's built-in FakeStore path.
        "--fake-process-group",
        "--disable-jit-fuser",
        "--transformer-impl",
        "local",
    ]
    merged = _merge_argv(defaults, filtered)
    sys.argv = [str(script), *merged]
    print(f"[probing.fakes] exec {script}", flush=True)
    try:
        runpy.run_path(str(script), run_name="__main__")
    except SystemExit as exc:
        code = exc.code
        return int(code) if isinstance(code, int) else (0 if code is None else 1)
    return 0


def _merge_argv(defaults: list[str], overrides: list[str]) -> list[str]:
    """Keep defaults, then apply overrides (last flag wins for valued options)."""
    out = list(defaults)
    i = 0
    while i < len(overrides):
        tok = overrides[i]
        if not tok.startswith("--"):
            i += 1
            continue
        # Valued option: --flag value (value does not start with --)
        if i + 1 < len(overrides) and not overrides[i + 1].startswith("--"):
            j = 0
            while j < len(out):
                if (
                    out[j] == tok
                    and j + 1 < len(out)
                    and not out[j + 1].startswith("--")
                ):
                    del out[j : j + 2]
                else:
                    j += 1
            out.extend([tok, overrides[i + 1]])
            i += 2
            continue
        # Boolean / store_true flag
        if tok not in out:
            out.append(tok)
        i += 1
    return out


if __name__ == "__main__":
    raise SystemExit(main())
