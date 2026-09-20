#!/usr/bin/env bash
# Run the development build with the macOS SDK compatible with this machine's
# installed linker. Extra arguments are forwarded to `cargo run`.

set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
cd "$repo_root"

export SDKROOT="$(xcrun --sdk macosx26.5 --show-sdk-path)"

exec cargo run "$@"
