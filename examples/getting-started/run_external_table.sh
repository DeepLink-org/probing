#!/usr/bin/env bash
# ExternalTable API 冒烟。
#
#   ./examples/getting-started/run_external_table.sh
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
[[ -f .venv/bin/activate ]] && source .venv/bin/activate
export PROBING="${PROBING:-1}"
exec python examples/getting-started/external_table.py "$@"
