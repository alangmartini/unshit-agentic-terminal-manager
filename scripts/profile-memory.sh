#!/usr/bin/env bash
# Thin wrapper. The real implementation lives in `xtask/` and is invoked via
# `cargo xtask profile memory` or the `cargo profile-memory` alias.
#
# Output: target/profile/dhat-heap.json
# Load the JSON at scripts/profile.html or https://nnethercote.github.io/dh_view/dh_view.html.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

ARGS=()
if [ $# -ge 1 ] && [ -n "${1:-}" ]; then
  ARGS+=(--out-dir "$1")
fi

exec cargo xtask profile memory "${ARGS[@]}"
