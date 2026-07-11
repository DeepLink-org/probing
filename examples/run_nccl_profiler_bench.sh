#!/usr/bin/env bash
# Run NCCL AllReduce benchmark twice (baseline vs probing-nccl-profiler) and
# print overhead. Requires Linux, CUDA, PyTorch, and a built plugin (.so).
#
# Usage:
#   ./examples/run_nccl_profiler_bench.sh
#   NPROC=8 BENCH_ITERS=500 ./examples/run_nccl_profiler_bench.sh
#
# Env:
#   NPROC          torchrun --nproc_per_node (default: visible GPU count or 2)
#   BENCH_ITERS    timed iterations per run (default 200)
#   WARMUP_ITERS   warmup iterations (default 20)
#   MSG_BYTES      message size (default 1048576)
#   OUT_DIR        JSON output directory (default /tmp/probing_nccl_bench_<pid>)

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "NCCL profiler benchmark requires Linux (found $(uname -s))" >&2
  exit 1
fi

if ! python -c "import torch; assert torch.cuda.is_available()" 2>/dev/null; then
  echo "CUDA + PyTorch required. Install torch in the project venv first." >&2
  exit 1
fi

# Build plugin if missing.
if ! python -m probing.nccl --plugin-path >/dev/null 2>&1; then
  echo "Building nccl profiler plugin..."
  make nccl-profiler-lib
fi

NPROC="${NPROC:-$(python -c "import torch; print(max(1, torch.cuda.device_count()))")}"
BENCH_ITERS="${BENCH_ITERS:-200}"
WARMUP_ITERS="${WARMUP_ITERS:-20}"
MSG_BYTES="${MSG_BYTES:-1048576}"
OUT_DIR="${OUT_DIR:-/tmp/probing_nccl_bench_$$}"
mkdir -p "$OUT_DIR"

BASELINE_JSON="$OUT_DIR/baseline.json"
PROFILED_JSON="$OUT_DIR/profiled.json"

COMMON_ARGS=(
  examples/nccl_profiler_overhead.py
  --warmup-iters "$WARMUP_ITERS"
  --bench-iters "$BENCH_ITERS"
  --msg-bytes "$MSG_BYTES"
)

echo "=== probing NCCL profiler overhead benchmark ==="
echo "nproc=$NPROC iters=$BENCH_ITERS msg_bytes=$MSG_BYTES out=$OUT_DIR"
echo

echo "--- baseline (no NCCL_PROFILER_PLUGIN) ---"
env -u NCCL_PROFILER_PLUGIN -u PROBING \
  torchrun --nproc_per_node="$NPROC" "${COMMON_ARGS[@]}" --output "$BASELINE_JSON"

echo
echo "--- profiled (probing-nccl-profiler) ---"
export NCCL_PROFILER_PLUGIN
NCCL_PROFILER_PLUGIN="$(python -m probing.nccl --plugin-path)"
export NCCL_PROFILE_EVENT_MASK
NCCL_PROFILE_EVENT_MASK="$(python -m probing.nccl --event-mask)"
export PROBING=2
export PROBING_DATA_DIR="${PROBING_DATA_DIR:-$OUT_DIR/probing_data}"
export PROBING_NCCL_INFLIGHT_THRESHOLD_SECS=0
mkdir -p "$PROBING_DATA_DIR"

torchrun --nproc_per_node="$NPROC" "${COMMON_ARGS[@]}" --output "$PROFILED_JSON"

echo
python examples/nccl_profiler_overhead.py --compare "$BASELINE_JSON" "$PROFILED_JSON"

echo
echo "Raw JSON: $BASELINE_JSON $PROFILED_JSON"
