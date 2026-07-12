#!/usr/bin/env bash
# vLLM / vLLM-Metal offline inference + probing soak (real vLLM; Web UI via PROBING_PORT).
#
# Prerequisites:
#   make develop
#   Linux/CUDA: uv pip install vllm
#   macOS: install vllm-metal — https://github.com/vllm-project/vllm-metal
#
# Usage:
#   ./examples/run_vllm_soak.sh
#   DURATION_SEC=120 ./examples/run_vllm_soak.sh
#   VLLM_MODEL=facebook/opt-125m ./examples/run_vllm_soak.sh
#
# Browser: http://127.0.0.1:${PROBING_PORT}/
#
# Mock contract tests: make test-python-regression  (tests/regression/ext/test_vllm_contract.py)

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

[[ -f .venv/bin/activate ]] && source .venv/bin/activate
PYTHON="${PYTHON:-python}"

if ! "$PYTHON" python/probing/dev_pth.py status >/dev/null 2>&1; then
  echo "error: dev env not ready — run: make develop" >&2
  exit 1
fi

_preflight_err="$(mktemp)"
trap 'rm -f "$_preflight_err"' EXIT

if ! PROBING=0 "$PYTHON" - "$ROOT" <<'PY' 2>"$_preflight_err"; then
import os
import sys

root = sys.argv[1]
venv_site = os.path.join(
    root,
    ".venv",
    "lib",
    f"python{sys.version_info.major}.{sys.version_info.minor}",
    "site-packages",
)


def _in_venv(path: str) -> bool:
    return not path or venv_site in path


def check_import(name: str, import_name: str | None = None) -> bool:
    import_name = import_name or name
    try:
        mod = __import__(import_name)
    except Exception as exc:
        print(f"{name}: {exc}", file=sys.stderr)
        return False
    path = getattr(mod, "__file__", "") or ""
    if not _in_venv(path):
        print(f"{name} loads from outside .venv: {path}", file=sys.stderr)
        print(f"  hint: uv pip install {name}", file=sys.stderr)
        return False
    return True


ok = check_import("torch") and check_import("torchvision") and check_import("transformers")
if not ok:
    sys.exit(1)

try:
    from vllm import LLM  # noqa: F401
except Exception as exc:
    print(f"vllm.LLM import failed: {exc}", file=sys.stderr)
    msg = str(exc)
    if "torchvision::nms" in msg:
        print(
            "  hint: torch/torchvision mismatch — reinstall matching wheels in .venv:",
            file=sys.stderr,
        )
        print(
            "    uv pip install torchvision torchaudio 'transformers>=4.46,<5'",
            file=sys.stderr,
        )
    sys.exit(1)
PY
  echo "error: vLLM import preflight failed" >&2
  if [[ -s "$_preflight_err" ]]; then
    sed 's/^/  /' "$_preflight_err" >&2
  fi
  if grep -q "outside .venv" "$_preflight_err" 2>/dev/null; then
    echo "  note: .venv uses --system-site-packages; pyenv global packages can shadow venv." >&2
    echo "        Install the full torch stack inside .venv (see examples/README.md)." >&2
  fi
  if [[ "$(uname -s)" == Darwin ]]; then
    echo "  macOS: uv pip install torchvision torchaudio 'transformers>=4.46,<5'" >&2
    echo "         plus vllm-metal — https://github.com/vllm-project/vllm-metal" >&2
  else
    echo "  Linux: uv pip install vllm torchvision 'transformers>=4.46,<5'" >&2
  fi
  exit 1
fi

DURATION_SEC="${DURATION_SEC:-600}"
MAX_BATCHES="${MAX_BATCHES:-0}"
PROBING_PORT="${PROBING_PORT:-18081}"
PROBING_DATA_DIR="${PROBING_DATA_DIR:-/tmp/probing_vllm_soak_$$}"
BATCH_SLEEP_MS="${BATCH_SLEEP_MS:-0}"
PRINT_FREQ="${PRINT_FREQ:-5}"

export PROBING="${PROBING:-2}"
export PROBING_VLLM="${PROBING_VLLM:-on}"
export PROBING_VLLM_STEP_SYNC="${PROBING_VLLM_STEP_SYNC:-on}"
export PROBING_PORT
export PROBING_DATA_DIR
export PROBING_RETENTION_DAYS="${PROBING_RETENTION_DAYS:-1}"
export PROBING_TORCH_PROFILING="${PROBING_TORCH_PROFILING:-on,shadow=4:1}"

export VLLM_USE_V1="${VLLM_USE_V1:-1}"

if [[ "$(uname -s)" == Darwin ]]; then
  export VLLM_METAL_USE_MLX="${VLLM_METAL_USE_MLX:-1}"
  export VLLM_MLX_DEVICE="${VLLM_MLX_DEVICE:-gpu}"
  export VLLM_WORKER_MULTIPROC_METHOD="${VLLM_WORKER_MULTIPROC_METHOD:-spawn}"
  # vllm-metal multi-prompt batched decode needs mlx_lm.BatchKVCache.merge (bleeding-edge).
  export VLLM_PROMPTS_PER_BATCH="${VLLM_PROMPTS_PER_BATCH:-1}"
  : "${VLLM_MODEL:=mlx-community/Qwen2.5-0.5B-Instruct-4bit}"
else
  export VLLM_PROMPTS_PER_BATCH="${VLLM_PROMPTS_PER_BATCH:-4}"
  : "${VLLM_MODEL:=facebook/opt-125m}"
fi
export VLLM_MODEL

mkdir -p "$PROBING_DATA_DIR"

COMMON_ARGS=(
  examples/vllm_offline_soak.py
  --model "$VLLM_MODEL"
  --max-duration-sec "$DURATION_SEC"
  --batch-sleep-ms "$BATCH_SLEEP_MS"
  --print-freq "$PRINT_FREQ"
)

if [[ "$MAX_BATCHES" -gt 0 ]]; then
  COMMON_ARGS+=(--max-batches "$MAX_BATCHES")
fi

echo "=== vLLM + probing soak (real inference) ==="
echo "PROBING=$PROBING  PROBING_PORT=$PROBING_PORT  VLLM_MODEL=$VLLM_MODEL"
echo "limits: DURATION_SEC=$DURATION_SEC  MAX_BATCHES=${MAX_BATCHES:-0}  VLLM_PROMPTS_PER_BATCH=${VLLM_PROMPTS_PER_BATCH:-?}"
echo "Web UI: http://127.0.0.1:${PROBING_PORT}/"
echo

exec "$PYTHON" "${COMMON_ARGS[@]}" "$@"
