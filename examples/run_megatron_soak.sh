#!/usr/bin/env bash
# Megatron-Core + probing soak (real megatron-core training loop; Web UI via PROBING_PORT).
#
# Prerequisites:
#   make develop
#   uv pip install megatron-core torch   # CUDA build
#   Linux + NVIDIA GPU(s); default uses 2 ranks with TP=2
#
# Usage:
#   ./examples/run_megatron_soak.sh
#   DURATION_SEC=120 ./examples/run_megatron_soak.sh
#   NPROC=1 TP_SIZE=1 ./examples/run_megatron_soak.sh
#
# Browser (while training): http://127.0.0.1:${PROBING_PORT}/
#
# Mock contract tests: make test-python-regression  (tests/regression/ext/test_megatron_contract.py)

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

[[ -f .venv/bin/activate ]] && source .venv/bin/activate
PYTHON="${PYTHON:-python}"

if ! "$PYTHON" python/probing/dev_pth.py status >/dev/null 2>&1; then
  echo "error: dev env not ready — run: make develop" >&2
  exit 1
fi

_os="$(uname -s)"
if [[ "$_os" != "Linux" ]]; then
  echo "error: Megatron-Core soak requires Linux + NVIDIA CUDA (current OS: $_os)" >&2
  echo "  On macOS use ./examples/run_vllm_soak.sh for a real-framework probing demo." >&2
  echo "  Megatron contract tests (no megatron-core): make test-python-regression" >&2
  exit 1
fi

if ! "$PYTHON" -m pip show megatron-core >/dev/null 2>&1; then
  echo "error: megatron-core package not found in this venv" >&2
  echo "  uv pip install megatron-core" >&2
  exit 1
fi

_import_err="$(
  PROBING=0 "$PYTHON" -c "import megatron.core" 2>&1
)" || {
  echo "error: megatron-core is installed but failed to import" >&2
  if [[ -n "$_import_err" ]]; then
    echo "$_import_err" | sed 's/^/  /' >&2
  fi
  if grep -q "No module named 'triton'" <<<"$_import_err"; then
    echo "  hint: install CUDA PyTorch + triton (Linux GPU only), e.g.:" >&2
    echo "    uv pip install torch triton megatron-core" >&2
  fi
  exit 1
}

"$PYTHON" -c "import torch; assert torch.cuda.is_available()" 2>/dev/null || {
  echo "error: CUDA torch required for Megatron-Core example" >&2
  echo "  install a CUDA build of torch and ensure nvidia-smi works" >&2
  exit 1
}

DURATION_SEC="${DURATION_SEC:-600}"
TRAIN_ITERS="${TRAIN_ITERS:-0}"
NPROC="${NPROC:-2}"
TP_SIZE="${TP_SIZE:-2}"
PP_SIZE="${PP_SIZE:-1}"
PROBING_PORT="${PROBING_PORT:-18080}"
PROBING_DATA_DIR="${PROBING_DATA_DIR:-/tmp/probing_megatron_soak_$$}"
STEP_SLEEP_MS="${STEP_SLEEP_MS:-0}"
PRINT_FREQ="${PRINT_FREQ:-10}"
MASTER_PORT="${MASTER_PORT:-29581}"

export PROBING="${PROBING:-2}"
export PROBING_MEGATRON="${PROBING_MEGATRON:-on}"
export PROBING_MEGATRON_STEP_SYNC="${PROBING_MEGATRON_STEP_SYNC:-on}"
export PROBING_PORT
export PROBING_DATA_DIR
export PROBING_RETENTION_DAYS="${PROBING_RETENTION_DAYS:-1}"
export PROBING_TORCH_PROFILING="${PROBING_TORCH_PROFILING:-on,shadow=4:1}"

mkdir -p "$PROBING_DATA_DIR"

COMMON_ARGS=(
  examples/megatron_mcore_train_loop.py
  --tensor-model-parallel-size "$TP_SIZE"
  --pipeline-model-parallel-size "$PP_SIZE"
  --max-duration-sec "$DURATION_SEC"
  --step-sleep-ms "$STEP_SLEEP_MS"
  --print-freq "$PRINT_FREQ"
  --skip-checkpoint
)

if [[ "$TRAIN_ITERS" -gt 0 ]]; then
  COMMON_ARGS+=(--train-iters "$TRAIN_ITERS")
fi

if [[ "$NPROC" -lt $((TP_SIZE * PP_SIZE)) ]]; then
  echo "error: NPROC=$NPROC < TP_SIZE*PP_SIZE=$((TP_SIZE * PP_SIZE))" >&2
  exit 1
fi

echo "=== Megatron-Core + probing soak ==="
echo "PROBING=$PROBING  PROBING_PORT=$PROBING_PORT  NPROC=$NPROC  TP=$TP_SIZE PP=$PP_SIZE"
echo "limits: DURATION_SEC=$DURATION_SEC  TRAIN_ITERS=${TRAIN_ITERS:-0}"
echo "Web UI: http://127.0.0.1:${PROBING_PORT}/"
echo

exec torchrun --standalone --nproc_per_node="$NPROC" \
  --master_port="$MASTER_PORT" \
  "$PYTHON" "${COMMON_ARGS[@]}" "$@"
