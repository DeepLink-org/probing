#!/usr/bin/env bash
# 本机模拟 N 机 × M 卡：自动并行启动 N 个 torchrun（每个 node_rank 一个）。
#
#   ./examples/run_cluster_multinode.sh           # 默认 2 机 × 2 卡
#   ./examples/run_cluster_multinode.sh 3 4       # 3 机 × 4 卡
#   ./examples/run_cluster_multinode.sh 2 2 60    # sleep 60s
#
# 也可用环境变量：NNODES NPROC SLEEP_SEC MASTER_PORT
# probing 高级开关（一般不用改）：PROBING_PORT PROBING_CLUSTER_REPORT=0
# 心跳预设：PROBING_CLUSTER_PRESET=demo|fast|steady（见 docs/src/design/torchrun-cluster.zh.md）
set -euo pipefail

cd "$(dirname "$0")/.."
[[ -f .venv/bin/activate ]] && source .venv/bin/activate

apply_cluster_preset() {
  local preset="${PROBING_CLUSTER_PRESET:-demo}"
  case "$preset" in
    demo)
      : # defaults: interval=10, stale=25, backoff on
      ;;
    fast)
      export PROBING_CLUSTER_REPORT_INTERVAL_SEC="${PROBING_CLUSTER_REPORT_INTERVAL_SEC:-5}"
      export PROBING_CLUSTER_STALE_SEC="${PROBING_CLUSTER_STALE_SEC:-30}"
      export PROBING_CLUSTER_REPORT_MAX_INTERVAL_SEC="${PROBING_CLUSTER_REPORT_MAX_INTERVAL_SEC:-22}"
      ;;
    steady)
      export PROBING_CLUSTER_STALE_SEC="${PROBING_CLUSTER_STALE_SEC:-90}"
      export PROBING_CLUSTER_REPORT_MAX_INTERVAL_SEC="${PROBING_CLUSTER_REPORT_MAX_INTERVAL_SEC:-67}"
      export PROBING_CLUSTER_REPORT_INTERVAL_SEC="${PROBING_CLUSTER_REPORT_INTERVAL_SEC:-15}"
      ;;
    *)
      echo "unknown PROBING_CLUSTER_PRESET=$preset (use demo|fast|steady)" >&2
      exit 1
      ;;
  esac
}

apply_cluster_preset

#  positional: N [M [sleep_sec]]
NNODES="${1:-${NNODES:-2}}"
NPROC="${2:-${NPROC:-2}}"
SLEEP_SEC="${3:-${SLEEP_SEC:-120}}"

MASTER_ADDR="${MASTER_ADDR:-127.0.0.1}"
MASTER_PORT="${MASTER_PORT:-29680}"
STAGGER_SEC="${STAGGER_SEC:-0.3}"
# All torchrun launches must share one rendezvous id (each `torchrun` otherwise gets a random id).
RDZV_ID="${RDZV_ID:-probing-${MASTER_PORT}}"
NODE0_BOOT_SEC="${NODE0_BOOT_SEC:-5}"

# PROBING=2：torchrun 子进程自动注入（Rust ctor 默认启动 cluster heartbeat）
export PROBING="${PROBING:-2}"
export PROBING_PORT="${PROBING_PORT:-18080}"
export MASTER_ADDR MASTER_PORT SLEEP_SEC RDZV_ID

if [[ "$(uname -s)" == Darwin ]]; then
  export GLOO_SOCKET_IFNAME="${GLOO_SOCKET_IFNAME:-lo0}"
fi

python -c "import torch" 2>/dev/null || {
  echo "need torch: uv pip install torch" >&2
  exit 1
}

if ! [[ "$NNODES" =~ ^[1-9][0-9]*$ ]] || ! [[ "$NPROC" =~ ^[1-9][0-9]*$ ]]; then
  echo "usage: $0 [NNODES [NPROC [SLEEP_SEC]]]" >&2
  exit 1
fi

WORLD=$((NNODES * NPROC))
echo "==> 启动 ${NNODES} 个 torchrun（每机 ${NPROC} rank，world_size=${WORLD}）"
echo "    MASTER=${MASTER_ADDR}:${MASTER_PORT}  RDZV_ID=${RDZV_ID}  probing=${PROBING_PORT}  SLEEP=${SLEEP_SEC}s"
echo "    cluster preset=${PROBING_CLUSTER_PRESET:-demo}  stale=${PROBING_CLUSTER_STALE_SEC:-25}s  interval=${PROBING_CLUSTER_REPORT_INTERVAL_SEC:-10}s"
echo "    运行中查询: probing -t ${MASTER_ADDR}:${PROBING_PORT} cluster nodes"

wait_for_master_port() {
  local deadline=$((SECONDS + NODE0_BOOT_SEC))
  while (( SECONDS < deadline )); do
    if python - <<PY 2>/dev/null
import socket
s = socket.socket()
s.settimeout(0.3)
try:
    s.connect(("${MASTER_ADDR}", ${MASTER_PORT}))
except OSError:
    raise SystemExit(1)
finally:
    s.close()
PY
    then
      return 0
    fi
    sleep 0.2
  done
  echo "    警告: ${MASTER_ADDR}:${MASTER_PORT} 在 ${NODE0_BOOT_SEC}s 内未就绪，后续节点可能 rendezvous 超时" >&2
  return 1
}

pids=()
cleanup() {
  local pid
  for pid in "${pids[@]:-}"; do
    kill "$pid" 2>/dev/null || true
  done
}
trap cleanup EXIT INT TERM

for node_rank in $(seq 0 $((NNODES - 1))); do
  if [[ "$node_rank" -gt 0 ]]; then
    wait_for_master_port || true
  fi
  echo "    torchrun #${node_rank}  (--node_rank=${node_rank})"
  torchrun \
    --nnodes="${NNODES}" \
    --node_rank="${node_rank}" \
    --nproc_per_node="${NPROC}" \
    --master_addr="${MASTER_ADDR}" \
    --master_port="${MASTER_PORT}" \
    --rdzv_id="${RDZV_ID}" \
    --local_addr=127.0.0.1 \
    examples/cluster_multinode_demo.py 2>&1 | sed -u "s/^/[node${node_rank}] /" &
  pids+=("$!")
  if [[ "$node_rank" -eq 0 ]]; then
    sleep "${NODE0_BOOT_SEC}"
  else
    sleep "${STAGGER_SEC}"
  fi
done

echo "==> 等待 ${NNODES} 个 torchrun 结束 ..."
fail=0
for i in "${!pids[@]}"; do
  if ! wait "${pids[$i]}"; then
    echo "    node_rank=${i} 退出非 0" >&2
    fail=1
  fi
done

if [[ "$fail" -ne 0 ]]; then
  echo "==> 有节点失败" >&2
  exit 1
fi
echo "==> 全部结束"

if command -v probing >/dev/null 2>&1; then
  echo "--- probing cluster nodes (global rank0) ---"
  probing -t "127.0.0.1:${PROBING_PORT}" cluster nodes 2>/dev/null || true
fi
