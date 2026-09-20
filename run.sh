#!/usr/bin/env bash
# Run the development build with the macOS SDK compatible with this machine's
# installed linker. Extra arguments are forwarded to `cargo run`.

set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
cd "$repo_root"

if [[ "$(uname -s)" == Darwin && -z "${SDKROOT:-}" ]]; then
    # Prefer the SDK compatible with this development machine when installed;
    # other Macs use their selected SDK, and explicit SDKROOT always wins.
    SDKROOT="$(xcrun --sdk macosx26.5 --show-sdk-path 2>/dev/null || xcrun --sdk macosx --show-sdk-path)"
    export SDKROOT
fi

exec cargo run "$@"
