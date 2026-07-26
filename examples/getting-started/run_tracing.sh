#!/usr/bin/env bash
# Tracing 入门 — span / phase hooks。
#
#   ./examples/getting-started/run_tracing.sh
#   PROBING_SPAN_BACKENDS=memtable,logger ./examples/getting-started/run_tracing.sh
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
[[ -f .venv/bin/activate ]] && source .venv/bin/activate
export PROBING="${PROBING:-1}"
exec python examples/getting-started/tracing.py "$@"
