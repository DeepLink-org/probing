#!/usr/bin/env bash
# 单进程 crash demo。
#
#   ./examples/crash/run_demo.sh
#   ./examples/crash/run_demo.sh exception
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
[[ -f .venv/bin/activate ]] && source .venv/bin/activate
export PROBING="${PROBING:-1}"
export PROBING_CRASH_NO_GRACE="${PROBING_CRASH_NO_GRACE:-1}"
MODE="${1:-record}"
shift || true
exec python examples/crash/crash_demo.py --mode "$MODE" "$@"
