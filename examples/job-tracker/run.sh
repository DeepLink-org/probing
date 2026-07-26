#!/usr/bin/env bash
# 作业开始/结束 hook 演示。
#
#   ./examples/job-tracker/run.sh
#   ./examples/job-tracker/run.sh via-init
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
[[ -f .venv/bin/activate ]] && source .venv/bin/activate
MODE="${1:-hook}"
case "$MODE" in
  via-init)
    exec python examples/job-tracker/job_tracker_via_init.py
    ;;
  *)
    exec python examples/job-tracker/job_tracker.py
    ;;
esac
