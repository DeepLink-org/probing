#!/usr/bin/env bash
# Torch module hooks 演示（ExternalTable 写入）。
#
#   ./examples/getting-started/run_hooks.sh
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
[[ -f .venv/bin/activate ]] && source .venv/bin/activate
export PROBING="${PROBING:-1}"
exec python examples/getting-started/hooks.py "$@"
