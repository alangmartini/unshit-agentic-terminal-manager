#!/usr/bin/env bash
# Thin wrapper. The real implementation lives in `xtask/` and is invoked via
# `cargo xtask profile all` or the `cargo profile-all` alias.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

ARGS=()
if [ $# -ge 1 ] && [ -n "${1:-}" ]; then
  ARGS+=(--out-dir "$1")
fi

exec cargo xtask profile all "${ARGS[@]}"
