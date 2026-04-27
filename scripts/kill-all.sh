#!/usr/bin/env bash
# Kill the terminal-manager UI and the unshit-ptyd session daemon.
#
# The daemon outlives the UI by design (sessions survive restarts), so
# the OS close button only stops the UI and the daemon keeps running
# with every previous PTY attached. When you need a clean slate (e.g.
# after changing the default shell, after a daemon crash, or before
# re-running `cargo run --release` to pick up new daemon code), call
# this script.

set -u

quiet=0
for arg in "$@"; do
    case "$arg" in
        -q|--quiet) quiet=1 ;;
    esac
done

log() {
    [ "$quiet" -eq 1 ] || echo "$*"
}

kill_target() {
    local name="$1"
    local pids

    if [[ "$OSTYPE" == "msys" || "$OSTYPE" == "cygwin" || "$OSTYPE" == "win32" ]]; then
        # Windows: ask PowerShell so we don't fight Git Bash arg mangling.
        pids=$(powershell.exe -NoProfile -Command "Get-Process -Name '$name' -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Id" 2>/dev/null | tr -d '\r')
    else
        pids=$(pgrep -x "$name" || true)
    fi

    if [ -z "$pids" ]; then
        log "no $name running"
        return 0
    fi

    for pid in $pids; do
        if kill -9 "$pid" 2>/dev/null; then
            log "killed $name pid=$pid"
        else
            echo "warning: failed to kill $name pid=$pid" >&2
        fi
    done
}

kill_target "terminal-manager"
kill_target "unshit-ptyd"

log "done"
