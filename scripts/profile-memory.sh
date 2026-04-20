#!/usr/bin/env bash
# Heap profile run: builds terminal-manager with the `profiling` cargo feature
# (dhat global allocator) and launches the app. When the app exits normally,
# dhat writes `target/profile/dhat-heap.json` with allocation backtraces.
#
# Load the JSON in the dashboard at scripts/profile.html, or directly at
# https://nnethercote.github.io/dh_view/dh_view.html.
set -euo pipefail

OUT_DIR="${1:-target/profile}"
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

mkdir -p "$OUT_DIR"

echo "==> cargo build --release --features profiling"
cargo build --release --features profiling

case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*) BIN="target/release/terminal-manager.exe" ;;
  *) BIN="target/release/terminal-manager" ;;
esac

if [ ! -x "$BIN" ]; then
  echo "binary not found at $BIN" >&2
  exit 1
fi

HEAP_FILE="$OUT_DIR/dhat-heap.json"
rm -f "$HEAP_FILE"

echo
echo "==> Launching app with dhat heap profiling."
echo "    Exercise the UI, then close the window to flush the profile."
echo

"$BIN"

echo
if [ -f "$HEAP_FILE" ]; then
  SIZE=$(wc -c <"$HEAP_FILE" | tr -d ' ')
  echo "==> Heap profile written: $HEAP_FILE ($SIZE bytes)"
  echo "    Open scripts/profile.html (Memory card) and drop the file on the viewer."
else
  echo "WARNING: expected $HEAP_FILE but the file was not produced." >&2
  echo "         Make sure the app closed via the window close button or Ctrl+C (not kill)." >&2
fi
