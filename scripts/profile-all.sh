#!/usr/bin/env bash
# Convenience wrapper: runs the CPU profile pass and the memory profile pass
# back to back, then opens the dashboard in the default browser. Each pass
# launches the app; use the UI for a representative workload, then close the
# window to advance to the next pass.
set -euo pipefail

OUT_DIR="${1:-target/profile}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

"$SCRIPT_DIR/profile-cpu.sh" "$OUT_DIR"
"$SCRIPT_DIR/profile-memory.sh" "$OUT_DIR"

DASH="$SCRIPT_DIR/profile.html"
echo
echo "==> Opening $DASH"
case "$(uname -s)" in
  Darwin) open "$DASH" ;;
  MINGW*|MSYS*|CYGWIN*) cmd.exe //c start "" "$DASH" ;;
  *) xdg-open "$DASH" >/dev/null 2>&1 || echo "open $DASH in your browser" ;;
esac
