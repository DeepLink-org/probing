#!/usr/bin/env bash
# Launch the inference metrics demo with HTTP dashboard enabled.
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec env PROBING=1 PROBING_PORT="${PROBING_PORT:-8080}" \
  python "${SCRIPT_DIR}/sglang_inference_metrics_demo.py" "$@"
