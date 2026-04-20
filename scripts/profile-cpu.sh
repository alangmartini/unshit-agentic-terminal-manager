#!/usr/bin/env bash
# Thin wrapper. The real implementation lives in `xtask/` and is invoked via
# `cargo xtask profile cpu` or the `cargo profile-cpu` alias.
#
# Output: target/profile/cpu.json.gz
# Load at scripts/profile.html (CPU card) or https://profiler.firefox.com.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

ARGS=()
if [ $# -ge 1 ] && [ -n "${1:-}" ]; then
  ARGS+=(--out-dir "$1")
fi
if [ -n "${RATE:-}" ]; then
  ARGS+=(--rate "$RATE")
fi

exec cargo xtask profile cpu "${ARGS[@]}"
