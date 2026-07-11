#!/usr/bin/env bash
# Install the latest probing nightly wheel from GitHub Releases (tag: nightly).
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/DeepLink-org/probing/main/scripts/install-nightly.sh | bash
#   PROBING_GITHUB_REPO=DeepLink-org/probing PROBING_NIGHTLY_TAG=nightly ./scripts/install-nightly.sh
#
# Environment:
#   PYTHON              Python interpreter (default: python3)
#   PROBING_GITHUB_REPO owner/repo (default: DeepLink-org/probing)
#   PROBING_NIGHTLY_TAG release tag (default: nightly)

set -euo pipefail

REPO="${PROBING_GITHUB_REPO:-DeepLink-org/probing}"
TAG="${PROBING_NIGHTLY_TAG:-nightly}"
PYTHON="${PYTHON:-python3}"

if ! command -v "$PYTHON" >/dev/null 2>&1; then
  echo "error: Python not found ($PYTHON)" >&2
  exit 1
fi

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) platform_re='manylinux.*x86_64' ;;
  Linux-aarch64) platform_re='manylinux.*aarch64' ;;
  Darwin-arm64) platform_re='macosx.*arm64' ;;
  Darwin-x86_64) platform_re='macosx.*x86_64' ;;
  *)
    echo "error: unsupported platform $(uname -s)-$(uname -m)" >&2
    exit 1
    ;;
esac

py_tag="$("$PYTHON" -c 'import sys; print(f"cp{sys.version_info.major}{sys.version_info.minor}")')"
min_py="$("$PYTHON" -c 'import sys; print(sys.version_info.major * 10 + sys.version_info.minor)')"
if [ "$min_py" -lt 38 ]; then
  echo "error: probing wheels require Python 3.8+ (abi3); got $("$PYTHON" --version)" >&2
  exit 1
fi
api="https://api.github.com/repos/${REPO}/releases/tags/${TAG}"

echo "Fetching nightly release assets from ${REPO} (tag=${TAG})..." >&2

url="$("$PYTHON" - <<PY
import json
import re
import sys
import urllib.error
import urllib.request

api = "${api}"
py_tag = "${py_tag}"
platform_re = re.compile(r"${platform_re}")

# Wheels are PyO3 abi3 (cp38-abi3): one wheel per OS/arch, not per Python minor.
ABI3_RE = re.compile(r"cp3[89]|cp3\d{2}-abi3")

req = urllib.request.Request(api, headers={"Accept": "application/vnd.github+json"})
try:
    with urllib.request.urlopen(req, timeout=60) as resp:
        data = json.load(resp)
except urllib.error.HTTPError as e:
    if e.code == 404:
        sys.exit(
            "nightly release not found — wait for the Nightly Build workflow on main, "
            f"or open https://github.com/${REPO}/actions/workflows/nightly.yml"
        )
    raise

for asset in data.get("assets", []):
    name = asset.get("name", "")
    if not name.endswith(".whl") or not name.startswith("probing-"):
        continue
    if not ABI3_RE.search(name):
        continue
    if platform_re.search(name):
        print(asset["browser_download_url"])
        break
else:
    sys.exit(
        f"no abi3 wheel for ${platform_re.pattern} in nightly release — "
        "check https://github.com/" + "${REPO}" + "/releases/tag/nightly"
    )
PY
)"

echo "Installing ${url}" >&2
exec "$PYTHON" -m pip install --upgrade pip
"$PYTHON" -m pip install --force-reinstall "$url"
"$PYTHON" -c "import probing; print('probing', probing.VERSION)"
