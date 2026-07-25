#!/usr/bin/env bash
# Ray + probing tracing hook。
#
#   ./examples/ray/run_tracing.sh
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
[[ -f .venv/bin/activate ]] && source .venv/bin/activate
export PROBING="${PROBING:-1}"
exec python examples/ray/ray_tracing_example.py "$@"
