#!/usr/bin/env bash
# CPU profile run: wraps the release binary with samply, a sampling profiler
# that produces Gecko/Firefox Profiler format. Install samply once with
# `cargo install samply` (this script will do it for you if missing).
#
# Output: target/profile/cpu.json.gz
# Load at scripts/profile.html (CPU card) or https://profiler.firefox.com.
set -euo pipefail

OUT_DIR="${1:-target/profile}"
RATE="${RATE:-1000}"
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

if ! command -v samply >/dev/null 2>&1; then
  echo "==> samply not found. Installing via cargo..."
  cargo install samply
fi

mkdir -p "$OUT_DIR"

echo "==> cargo build --release"
cargo build --release

case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*) BIN="target/release/terminal-manager.exe" ;;
  *) BIN="target/release/terminal-manager" ;;
esac

if [ ! -x "$BIN" ]; then
  echo "binary not found at $BIN" >&2
  exit 1
fi

CPU_FILE="$OUT_DIR/cpu.json.gz"
rm -f "$CPU_FILE"

echo
echo "==> samply record --save-only --output $CPU_FILE --rate $RATE -- $BIN"
echo "    Exercise the UI, then close the window to stop recording."
echo

samply record --save-only --output "$CPU_FILE" --rate "$RATE" -- "$BIN"

echo
if [ -f "$CPU_FILE" ]; then
  SIZE=$(wc -c <"$CPU_FILE" | tr -d ' ')
  echo "==> CPU profile written: $CPU_FILE ($SIZE bytes)"
  echo "    Open scripts/profile.html (CPU card) and drop the file on the viewer."
else
  echo "WARNING: expected $CPU_FILE but the file was not produced." >&2
fi
