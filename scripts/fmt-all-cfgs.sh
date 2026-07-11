#!/usr/bin/env bash
# Run rustfmt for the host and for additional compilation targets so that
# platform-specific #[cfg(...)] branches (windows / linux / aarch64, etc.)
# are visited even when developing on a single OS.
#
# Usage:
#   ./scripts/fmt-all-cfgs.sh          # format
#   ./scripts/fmt-all-cfgs.sh --check  # check only
#
# Env:
#   FMT_EXTRA_TARGETS  space-separated triples (override defaults)
#   SKIP_WEB           set to 1 to skip web/

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

CHECK=()
if [[ "${1:-}" == "--check" ]]; then
  CHECK=(-- --check)
fi

HOST="$(rustc -vV | sed -n 's/^host: //p')"

# Default extra targets: skip any that match the current host.
DEFAULT_TARGETS=(
  x86_64-unknown-linux-gnu
  aarch64-unknown-linux-gnu
  x86_64-pc-windows-msvc
  aarch64-apple-darwin
  x86_64-apple-darwin
)

if [[ -n "${FMT_EXTRA_TARGETS:-}" ]]; then
  # shellcheck disable=SC2206
  TARGETS=($FMT_EXTRA_TARGETS)
else
  TARGETS=()
  for t in "${DEFAULT_TARGETS[@]}"; do
    [[ "$t" == "$HOST" ]] && continue
    TARGETS+=("$t")
  done
fi

run_fmt() {
  local label="$1"
  shift
  echo "==> cargo fmt ($label)"
  cargo fmt --all "$@"
  if [[ "${SKIP_WEB:-0}" != "1" ]]; then
    (cd web && cargo fmt --all "$@")
  fi
}

run_fmt "host=${HOST}" "${CHECK[@]}"

for target in "${TARGETS[@]}"; do
  if ! rustup target list --installed | rg -qx "$target"; then
    if ! rustup target add "$target" 2>/dev/null; then
      echo "==> skip target $target (not installable on this host)" >&2
      continue
    fi
  fi
  # CARGO_BUILD_TARGET makes rustfmt/cargo use that target's cfg set.
  CARGO_BUILD_TARGET="$target" run_fmt "target=${target}" "${CHECK[@]}"
done

echo "fmt-all-cfgs: done"
