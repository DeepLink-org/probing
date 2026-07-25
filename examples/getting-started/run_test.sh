#!/usr/bin/env bash
# 父子进程 / 嵌套 PROBING 冒烟。
#
#   ./examples/getting-started/run_test.sh
#   ./examples/getting-started/run_test.sh --depth 2
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
[[ -f .venv/bin/activate ]] && source .venv/bin/activate
export PROBING="${PROBING:-1}"
exec python examples/getting-started/test_probing.py "$@"
