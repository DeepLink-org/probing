"""Wheel / develop entry executed from ``probing.pth`` or ``probing_hook.pth``."""

from __future__ import annotations

import os
import sys


def _is_elastic_supervisor() -> bool:
    """Match Rust ``is_elastic_supervisor`` without importing ``probing``."""
    if os.environ.get("LOCAL_RANK") is not None or os.environ.get("RANK") is not None:
        return False
    if os.environ.get("TORCHELASTIC_RUN_ID"):
        return True
    if os.path.basename(sys.argv[0]) == "torchrun":
        return True
    # During site init for ``python -m <mod>``, CPython may only expose ``['-m', ...]``
    # before the module name is appended. Worker ranks re-exec with RANK/LOCAL_RANK set.
    if sys.argv[:1] == ["-m"]:
        if len(sys.argv) >= 2 and not sys.argv[1].startswith("-"):
            return False
        return True
    try:
        idx = sys.argv.index("-m")
        mod = sys.argv[idx + 1]
    except (ValueError, IndexError):
        mod = None
    if mod in ("torch.distributed.run", "torch.distributed.launch"):
        return True
    return any(
        (arg or "").endswith("torchrun") or "torch/distributed/run" in (arg or "")
        for arg in sys.argv
    )


_sup = _is_elastic_supervisor()
if not _sup:
    from probing.site_hook import run_site_hook

    run_site_hook()
