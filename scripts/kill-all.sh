#!/usr/bin/env bash
# Stop this repo's development UI and daemon. Installed copies, dist bundles,
# other checkouts, and processes with unknown executable paths are spared.
# Stopping a daemon ends its shells; use --dry-run to inspect targets first.

set -u

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"

executable_path() {
    local pid="$1" line
    case "$OSTYPE" in
        darwin*)
            # ps may report a relative argv[0]. The first text vnode is the
            # actual executable; later entries can be shared libraries.
            while IFS= read -r line; do
                case "$line" in n*) printf '%s\n' "${line#n}"; return ;; esac
            done < <(lsof -n -P -a -p "$pid" -d txt -Fn 2>/dev/null)
            ;;
        linux*) readlink "/proc/$pid/exe" 2>/dev/null ;;
    esac
}

is_repo_build() {
    case "$1" in
        "$repo_root"/target/*|"$repo_root"/target-*/*) return 0 ;;
        *) return 1 ;;
    esac
}

main() {
    local quiet=0 all=0 dry_run=0 arg name pid path current_path pids
    for arg in "$@"; do
        case "$arg" in
            -q|--quiet) quiet=1 ;;
            --all|-All) all=1 ;;
            --dry-run) dry_run=1 ;;
            -h|--help)
                echo 'Usage: scripts/kill-all.sh [--quiet] [--dry-run] [--all]'
                echo 'Default: only this repo target/ and target-*/ builds.'
                echo '--all: include installed copies and other checkouts (ends their sessions).'
                return 0 ;;
            *) echo "error: unknown argument: $arg" >&2; return 2 ;;
        esac
    done

    case "$OSTYPE" in
        msys*|cygwin*|win32*)
            if [[ "$dry_run" -eq 1 ]]; then
                echo 'error: --dry-run is supported on macOS and Linux only' >&2
                return 2
            fi
            # Use positional parameters for compatibility with macOS Bash 3.2.
            set --
            [[ "$quiet" -eq 0 ]] || set -- "$@" -Quiet
            [[ "$all" -eq 0 ]] || set -- "$@" -All
            powershell.exe -NoProfile -File "$repo_root/scripts/kill-all.ps1" "$@"
            return $? ;;
        darwin*|linux*) ;;
        *) echo "error: unsupported OS: $OSTYPE" >&2; return 2 ;;
    esac

    for name in terminal-manager unshit-ptyd; do
        pids="$(pgrep -x "$name" || true)"
        for pid in $pids; do
            [[ "$pid" =~ ^[0-9]+$ ]] || continue
            path="$(executable_path "$pid")"
            if [[ "$all" -eq 0 ]] && { ! is_repo_build "$path" || [[ "${path##*/}" != "$name" ]]; }; then
                [[ "$quiet" -eq 1 ]] || echo "spared $name pid=$pid (${path:-unknown path})"
                continue
            fi
            if [[ "$dry_run" -eq 1 ]]; then
                [[ "$quiet" -eq 1 ]] || echo "would kill $name pid=$pid (${path:-unknown path})"
                continue
            fi
            # Recheck immediately before signaling; fail closed if it changed.
            current_path="$(executable_path "$pid")"
            if [[ "$current_path" != "$path" ]]; then
                echo "warning: executable changed for pid=$pid; skipping" >&2
                continue
            fi
            if kill -9 "$pid" 2>/dev/null; then
                [[ "$quiet" -eq 1 ]] || echo "killed $name pid=$pid ($path)"
            else
                echo "warning: failed to kill $name pid=$pid" >&2
            fi
        done
    done
    return 0
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
    main "$@"
fi
