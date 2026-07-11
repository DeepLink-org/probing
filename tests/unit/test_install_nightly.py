"""Contract tests for scripts/install-nightly.sh wheel selection logic."""

from __future__ import annotations

import re
import subprocess
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = REPO_ROOT / "scripts" / "install-nightly.sh"

ABI3_RE = re.compile(r"cp3[89]|cp3\d{2}-abi3")


def _pick_wheel_url(
    assets: list[dict],
    *,
    platform_re: re.Pattern[str],
) -> str:
    """Mirror the selection logic embedded in install-nightly.sh."""
    for asset in assets:
        name = asset.get("name", "")
        if not name.endswith(".whl") or not name.startswith("probing-"):
            continue
        if not ABI3_RE.search(name):
            continue
        if platform_re.search(name):
            return asset["browser_download_url"]
    raise RuntimeError("no matching wheel")


@pytest.mark.parametrize(
    ("pattern", "expected_url"),
    [
        (re.compile(r"manylinux.*x86_64"), "https://example/linux.whl"),
        (re.compile(r"macosx.*arm64"), "https://example/mac.whl"),
    ],
)
def test_install_nightly_picks_abi3_wheel_for_platform(
    pattern: re.Pattern[str], expected_url: str
):
    assets = [
        {
            "name": "probing-0.2.5+g1.abc1234-cp38-abi3-manylinux_2_17_x86_64.whl",
            "browser_download_url": "https://example/linux.whl",
        },
        {
            "name": "probing-0.2.5+g1.abc1234-cp38-abi3-macosx_11_0_arm64.whl",
            "browser_download_url": "https://example/mac.whl",
        },
        {
            "name": "probing-0.2.5-cp312-cp312-manylinux_2_17_x86_64.whl",
            "browser_download_url": "https://example/wrong-tag.whl",
        },
    ]
    url = _pick_wheel_url(assets, platform_re=pattern)
    assert url == expected_url


def test_install_nightly_script_syntax():
    assert SCRIPT.is_file()
    subprocess.run(["bash", "-n", str(SCRIPT)], check=True)
