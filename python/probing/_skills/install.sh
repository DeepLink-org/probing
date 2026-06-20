#!/usr/bin/env bash
# Install repo skills/ into Cursor, Claude Code, and Codex skill directories.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if command -v probing >/dev/null 2>&1; then
  exec probing skill install "$@"
fi

export PROBING=1
export PYTHONPATH="${ROOT}/python${PYTHONPATH:+:${PYTHONPATH}}"
exec python3 -m probing.skills install "$@"
