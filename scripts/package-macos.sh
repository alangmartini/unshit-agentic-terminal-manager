#!/usr/bin/env bash
# Build a self-contained macOS application bundle.
#
# The UI discovers unshit-ptyd beside its own executable, so both binaries
# intentionally live in Contents/MacOS. The resulting bundle is ad-hoc signed
# when the local Command Line Tools provide codesign; a Developer ID identity
# can be applied separately for distribution.

set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

if [[ "${1:-}" == "--help" || "${1:-}" == "-h" ]]; then
	cat <<'USAGE'
Usage: scripts/package-macos.sh

Builds terminal-manager and unshit-ptyd in release mode and writes:
  dist/Terminal Manager.app

Set SDKROOT, MACOSX_DEPLOYMENT_TARGET, CARGO_TARGET_DIR, or
CARGO_BUILD_TARGET in the environment to customize the build.
USAGE
	exit 0
fi

if [[ $# -ne 0 ]]; then
	echo "error: this script does not accept positional arguments" >&2
	exit 2
fi

: "${CARGO:=cargo}"
: "${MACOSX_DEPLOYMENT_TARGET:=11.0}"
export MACOSX_DEPLOYMENT_TARGET

# The host used for development currently advertises a Command Line Tools SDK
# whose libSystem.tbd contains an architecture that this linker cannot parse.
# Validate SDKs with a tiny real link before choosing one, while preserving an
# explicitly supplied SDKROOT exactly (and failing clearly if it is unusable).
sdk_can_link() {
	local sdk="$1"
	local clang_bin probe_root source_file probe_binary

	[[ -d "$sdk" && -f "$sdk/usr/lib/libSystem.tbd" ]] || return 1
	clang_bin="$(command -v clang || true)"
	[[ -n "$clang_bin" ]] || return 1

	probe_root="$(mktemp -d "${TMPDIR:-/tmp}/terminal-manager-sdk.XXXXXX")" || return 1
	source_file="$probe_root/main.c"
	probe_binary="$probe_root/main"
	printf '%s\n' 'int main(void) { return 0; }' > "$source_file"
	if "$clang_bin" \
		-isysroot "$sdk" \
		-mmacosx-version-min="$MACOSX_DEPLOYMENT_TARGET" \
		"$source_file" -o "$probe_binary" >/dev/null 2>&1; then
		rm -rf -- "$probe_root"
		return 0
	fi
	rm -rf -- "$probe_root"
	return 1
}

if [[ -n "${SDKROOT:-}" ]]; then
	if ! sdk_can_link "$SDKROOT"; then
		echo "error: explicit SDKROOT is not a usable macOS SDK: $SDKROOT" >&2
		exit 1
	fi
	echo "Using SDKROOT=$SDKROOT (provided by the environment)"
else
	declare -a sdk_candidates=()
	developer_dir="$(xcode-select -p 2>/dev/null || true)"
	if [[ -n "$developer_dir" ]]; then
		for sdk in "$developer_dir/SDKs"/MacOSX*.sdk; do
			[[ -d "$sdk" ]] && sdk_candidates+=("$sdk")
		done
	fi
	for sdk in /Library/Developer/CommandLineTools/SDKs/MacOSX*.sdk; do
		[[ -d "$sdk" ]] && sdk_candidates+=("$sdk")
	done
	default_sdk="$(xcrun --sdk macosx --show-sdk-path 2>/dev/null || true)"
	if [[ -n "$default_sdk" ]]; then
		# Try the platform-selected SDK first, then fall back through installed
		# SDK directories in reverse glob order without relying on sort -V.
		sdk_candidates+=("$default_sdk")
	fi

	selected_sdk=""
	for ((index = ${#sdk_candidates[@]} - 1; index >= 0; index--)); do
		sdk="${sdk_candidates[$index]}"
		[[ -n "$sdk" ]] || continue
		if sdk_can_link "$sdk"; then
			selected_sdk="$sdk"
			break
		fi
	done
	if [[ -z "$selected_sdk" ]]; then
		echo "error: no usable macOS SDK found; set SDKROOT to a compatible SDK" >&2
		exit 1
	fi
	export SDKROOT="$selected_sdk"
	echo "Using SDKROOT=$SDKROOT (auto-selected after linker validation)"
fi

echo "Building terminal-manager and unshit-ptyd for macOS..."
"$CARGO" build \
	--locked \
	--release \
	-p terminal-manager --bin terminal-manager \
	-p unshit-ptyd --bin unshit-ptyd

target_dir="${CARGO_TARGET_DIR:-$repo_root/target}"
if [[ "$target_dir" != /* ]]; then
	target_dir="$repo_root/$target_dir"
fi
if [[ -n "${CARGO_BUILD_TARGET:-}" ]]; then
	binary_dir="$target_dir/$CARGO_BUILD_TARGET/release"
else
	binary_dir="$target_dir/release"
fi

ui_binary="$binary_dir/terminal-manager"
daemon_binary="$binary_dir/unshit-ptyd"
for binary in "$ui_binary" "$daemon_binary"; do
	if [[ ! -x "$binary" ]]; then
		echo "error: expected executable was not produced: $binary" >&2
		exit 1
	fi
done

dist_dir="$repo_root/dist"
bundle="$dist_dir/Terminal Manager.app"
stage_root="$(mktemp -d "${TMPDIR:-/tmp}/terminal-manager-app.XXXXXX")"
stage_bundle="$stage_root/Terminal Manager.app"

cleanup_stage() {
	if [[ -n "${stage_root:-}" && -d "$stage_root" ]]; then
		rm -rf -- "$stage_root"
	fi
}
trap cleanup_stage EXIT INT TERM

mkdir -p "$stage_bundle/Contents/MacOS" "$stage_bundle/Contents/Resources"
cp "$ui_binary" "$stage_bundle/Contents/MacOS/terminal-manager"
cp "$daemon_binary" "$stage_bundle/Contents/MacOS/unshit-ptyd"
cp "$repo_root/packaging/macos/Info.plist" "$stage_bundle/Contents/Info.plist"
cp "$repo_root/LICENSE" "$stage_bundle/Contents/Resources/LICENSE"

# Stamp the crate version into the staged bundle. The checked-in Info.plist
# only holds a placeholder; shipped unchanged, every release would report the
# same version in Finder and the About panel. `cargo pkgid` prints
# `path+file:///...#terminal-manager@0.6.1` (or `#0.6.1` when the package name
# matches the directory name), so the version is the text after the last `#`
# and `@`. A version that does not parse aborts the build rather than
# shipping the placeholder.
pkgid="$(cargo pkgid -p terminal-manager)"
version="${pkgid##*#}"
version="${version##*@}"
if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.-]+)?$ ]]; then
	echo "error: could not read the crate version from cargo pkgid output: $pkgid" >&2
	exit 1
fi

# Replace the <string> that follows each of the two version keys. Plain awk so
# the step also runs where plutil is unavailable; plutil -lint below validates
# the result on macOS.
stamp_plist_version() {
	local plist="$1" version="$2" tmp="$1.tmp"
	if ! awk -v version="$version" '
		stamp {
			if (sub(/<string>[^<]*<\/string>/, "<string>" version "</string>")) { stamped++ }
			stamp = 0
		}
		/<key>(CFBundleShortVersionString|CFBundleVersion)<\/key>/ { stamp = 1 }
		{ print }
		END {
			if (stamped != 2) {
				print "error: stamped " stamped + 0 " version strings in Info.plist, expected 2" > "/dev/stderr"
				exit 1
			}
		}
	' "$plist" > "$tmp"; then
		rm -f -- "$tmp"
		exit 1
	fi
	mv -- "$tmp" "$plist"
}
stamp_plist_version "$stage_bundle/Contents/Info.plist" "$version"
echo "Bundle version: $version"

make_icns() {
	local source_png iconset size double
	if ! command -v sips >/dev/null 2>&1 || ! command -v iconutil >/dev/null 2>&1; then
		echo "warning: sips/iconutil unavailable; packaging without an application icon" >&2
		return 0
	fi

	source_png="$stage_root/app.png"
	iconset="$stage_root/TerminalManager.iconset"
	mkdir -p "$iconset"
	if ! sips -s format png "$repo_root/packaging/app.ico" --out "$source_png" >/dev/null; then
		echo "warning: could not convert packaging/app.ico; packaging without an application icon" >&2
		return 0
	fi

	for size in 16 32 128 256 512; do
		double=$((size * 2))
		sips -z "$size" "$size" "$source_png" --out "$iconset/icon_${size}x${size}.png" >/dev/null
		sips -z "$double" "$double" "$source_png" --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
	done

	if ! iconutil --convert icns \
		--output "$stage_bundle/Contents/Resources/TerminalManager.icns" \
		"$iconset" >/dev/null; then
		echo "warning: could not create TerminalManager.icns; packaging without an application icon" >&2
		return 0
	fi
}

make_icns

if command -v plutil >/dev/null 2>&1; then
	plutil -lint "$stage_bundle/Contents/Info.plist" >/dev/null
fi

if command -v codesign >/dev/null 2>&1; then
	if ! codesign --force --deep --sign - "$stage_bundle" >/dev/null; then
		echo "warning: ad-hoc codesigning failed; the bundle is still usable for local testing" >&2
	fi
else
	echo "warning: codesign unavailable; the bundle is unsigned" >&2
fi

mkdir -p "$dist_dir"
# The destination is intentionally fixed to this repository's dist directory;
# this guard prevents an accidental variable/path change from broadening the
# cleanup target.
if [[ "$bundle" != "$dist_dir/Terminal Manager.app" ]]; then
	echo "error: refusing to replace an unexpected bundle path: $bundle" >&2
	exit 1
fi
if [[ -e "$bundle" || -L "$bundle" ]]; then
	rm -rf -- "$bundle"
fi
mv "$stage_bundle" "$bundle"

echo "Created: $bundle"
echo "Launch with: open \"$bundle\""
