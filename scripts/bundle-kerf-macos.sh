#!/usr/bin/env bash
# Lays out Industrial Kerf.app around a built kerf-editor, as macOS wants an application to
# be: the editor in Contents/MacOS, apps/kerf-editor/packaging/macos/Info.plist with this
# version in it (which names the editor, its icon, and kerf:// links, so Launch Services
# sends those to it), the icon in Contents/Resources, and the fonts' licences beside it.
# The editor itself is the one Rust binary cargo builds; nothing else goes in here.
#
# package-kerf-macos.sh makes the app it packs with it, and install-kerf-local.sh the one
# it installs in ~/Applications.
#
#   scripts/bundle-kerf-macos.sh path/to/kerf-editor "path/to/Industrial Kerf.app"
set -euo pipefail

[ $# -eq 2 ] || { echo "usage: $0 path/to/kerf-editor path/to/Industrial Kerf.app" >&2; exit 2; }
root="$(cd "$(dirname "$0")/.." && pwd)"
editor="$1"
app="$2"
[ -x "$editor" ] || { echo "No editor at $editor: cargo build --release --manifest-path apps/kerf-editor/Cargo.toml" >&2; exit 1; }
version="$(sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"/\1/p' "$root/Cargo.toml")"

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources/fonts"
cp -fL "$editor" "$app/Contents/MacOS/kerf-editor"
chmod 755 "$app/Contents/MacOS/kerf-editor"
sed "s/@VERSION@/$version/g" "$root/apps/kerf-editor/packaging/macos/Info.plist" > "$app/Contents/Info.plist"
plutil -lint "$app/Contents/Info.plist" > /dev/null
# The icon Finder and the Dock show, named by CFBundleIconFile (tools/kerf-icon makes it).
cp "$root/apps/kerf-editor/assets/icon/industrial-kerf.icns" "$app/Contents/Resources/"
cp "$root"/apps/kerf-editor/assets/fonts/*-LICENSE.txt "$app/Contents/Resources/fonts/"
