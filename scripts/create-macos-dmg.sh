#!/usr/bin/env bash
# Make the familiar drag-to-Applications disk image from an existing app bundle.

set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
	echo "error: macOS is required to build a DMG" >&2
	exit 1
fi

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
app="${1:-$repo_root/dist/Terminal Manager.app}"
if [[ ! -d "$app" || ! -f "$app/Contents/Info.plist" ]]; then
	echo "error: no application bundle at $app" >&2
	exit 1
fi
app="$(cd -- "$(dirname -- "$app")" && pwd -P)/$(basename -- "$app")"
/usr/bin/codesign --verify --deep --strict "$app"
version="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$app/Contents/Info.plist")"
archs="$(/usr/bin/lipo -archs "$app/Contents/MacOS/terminal-manager")"
if [[ "$archs" == *arm64* && "$archs" == *x86_64* ]]; then
	arch="universal"
elif [[ "$archs" == *arm64* ]]; then
	arch="arm64"
elif [[ "$archs" == *x86_64* ]]; then
	arch="x86_64"
else
	echo "error: unsupported application architecture: $archs" >&2
	exit 1
fi
output="${2:-$repo_root/dist/terminal-manager-$version-macos-$arch.dmg}"
mkdir -p -- "$(dirname -- "$output")"
output="$(cd -- "$(dirname -- "$output")" && pwd -P)/$(basename -- "$output")"

work="$(mktemp -d "${TMPDIR:-/tmp}/terminal-manager-dmg.XXXXXX")"
mountpoint="$work/mount"
mounted=0
cleanup() {
	if [[ "$mounted" == 1 ]]; then
		/usr/bin/hdiutil detach "$mountpoint" -quiet || true
	fi
	rm -rf -- "$work"
}
trap cleanup EXIT INT TERM

mkdir -p "$work/contents" "$mountpoint"
/usr/bin/ditto "$app" "$work/contents/Terminal Manager.app"
/usr/bin/codesign --verify --deep --strict "$work/contents/Terminal Manager.app"
ln -s /Applications "$work/contents/Applications"

volume_name="Drag Terminal Manager to Applications"
/usr/bin/hdiutil create -quiet -srcfolder "$work/contents" -fs HFS+ \
	-volname "$volume_name" -format UDRW "$work/layout.dmg"
/usr/bin/hdiutil attach -quiet -readwrite -noverify -noautoopen \
	-mountpoint "$mountpoint" "$work/layout.dmg"
mounted=1

# Finder writes icon positions and window settings into the volume's .DS_Store.
# Fail rather than publish an image without the requested installer window.
if [[ "${TM_DMG_CONFIGURE_FINDER:-1}" == 1 ]]; then
	if ! /usr/bin/osascript - "$mountpoint" <<'APPLESCRIPT'
on run argv
	set mountedPath to item 1 of argv
	set volumeFolder to (POSIX file mountedPath) as alias
	tell application "Finder"
		with timeout of 15 seconds
			open volumeFolder
			set theWindow to container window of volumeFolder
			set current view of theWindow to icon view
			set toolbar visible of theWindow to false
			set statusbar visible of theWindow to false
			set bounds of theWindow to {120, 120, 800, 520}
			set icon size of icon view options of theWindow to 112
			set text size of icon view options of theWindow to 13
			set position of item "Terminal Manager.app" of volumeFolder to {170, 190}
			set position of item "Applications" of volumeFolder to {500, 190}
			update volumeFolder without registering applications
			delay 1
			close theWindow
		end timeout
	end tell
end run
APPLESCRIPT
	then
		echo "error: Finder could not configure the drag-to-Applications window" >&2
		exit 1
	fi
	if [[ ! -s "$mountpoint/.DS_Store" ]]; then
		echo "error: Finder did not save the DMG window layout" >&2
		exit 1
	fi
fi

/usr/bin/hdiutil detach "$mountpoint" -quiet
mounted=0
/usr/bin/hdiutil convert "$work/layout.dmg" -quiet -format UDZO -ov -o "$output"
/usr/bin/hdiutil verify -quiet "$output"
echo "Created: $output"
