#!/usr/bin/env bash
# Install Kerf's command-line tools (kerf, kerfd, kerf-mcp) to ~/.local/bin,
# and the editor: on macOS as an app, ~/Applications/Industrial Kerf.app,
# which carries kerfd beside it; elsewhere as ~/.local/bin/kerf-editor, with
# its fonts' licences in ~/.local/lib/kerf. The editor is one Rust binary
# (apps/kerf-editor, drawn with GPUI), so the install does not refer back to
# this checkout, and cleaning or moving it is safe.
# System libraries (FFmpeg) still come from their package manager.
# Everything is built optimized: the editor's live preview and its exports
# draw frames in kerf-bridge, which it links, and it draws every frame of its
# motion.
set -euo pipefail

kerf_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$kerf_root"
dest="${KERF_LOCAL_BIN:-$HOME/.local/bin}"
lib_dir="${KERF_LOCAL_LIB:-$(dirname "$dest")/lib/kerf}"
apps_dir="${KERF_LOCAL_APPS:-$HOME/Applications}"
mkdir -p "$dest"

mode="${1:-all}"

build_cli() {
    cargo build --release -p kerf-cli --bin kerf
    cargo build --release -p kerf-runtime --bin kerfd
    cargo build --release -p kerf-mcp --bin kerf-mcp
}

build_editor() {
    cargo build --release --manifest-path apps/kerf-editor/Cargo.toml
}

# Copy a file over whatever is at the destination, including an older
# install's symlink into the checkout (which cp would write through).
place() {
    rm -f "$2"
    cp -fL "$1" "$2"
    chmod 755 "$2"
}

install_cli() {
    for binary in kerf kerfd kerf-mcp; do
        place "$kerf_root/target/release/$binary" "$dest/$binary"
    done
}

# What the Qt editor's install left: its binary and its bridge library.
remove_old_editor() {
    rm -f "$dest/kerf-qt" "$lib_dir/libkerf_qt_bridge.so" "$lib_dir/libkerf_qt_bridge.dylib" \
        "$lib_dir"/libqtadvanceddocking*.dylib
}

# The editor as an app, for Finder, the Dock and Spotlight, and its only
# install on macOS (kerf opens it too): a fresh bundle in place of an older
# install's (never copied over it), with kerfd beside its executable, where
# the editor looks first (an app opened from Finder has no shell PATH). It is
# signed again, ad hoc, as macOS refuses to run code whose signature no longer
# matches.
install_editor_macos() {
    local app="$apps_dir/Industrial Kerf.app"
    local kerfd="$dest/kerfd"
    [ -x "$kerfd" ] || kerfd="$kerf_root/target/release/kerfd"
    mkdir -p "$apps_dir"
    "$kerf_root/scripts/bundle-kerf-macos.sh" "$kerf_root/target/release/kerf-editor" "$app"
    if [ -x "$kerfd" ]; then
        place "$kerfd" "$app/Contents/MacOS/kerfd"
    else
        echo "warning: kerfd is not built; the app looks for kerfd on its PATH (run '$0 all')" >&2
    fi
    codesign --force --deep --sign - "$app" 2>/dev/null
    # What an older install left in ~/.local: kerf would open a stale editor
    # there before the app.
    rm -f "$dest/kerf-editor"
    remove_old_editor
    rmdir "$lib_dir" 2>/dev/null || true
}

install_editor() {
    case "$(uname -s)" in
        Darwin) install_editor_macos ;;
        *)
            place "$kerf_root/target/release/kerf-editor" "$dest/kerf-editor"
            mkdir -p "$lib_dir/fonts"
            cp -f "$kerf_root"/apps/kerf-editor/assets/fonts/*-LICENSE.txt "$lib_dir/fonts/"
            remove_old_editor
            ;;
    esac
}

case "$mode" in
    all)
        build_cli
        build_editor
        install_cli
        install_editor
        ;;
    cli)
        build_cli
        install_cli
        ;;
    editor)
        build_editor
        install_editor
        ;;
    *)
        echo "usage: $0 [all|cli|editor]" >&2
        exit 2
        ;;
esac

echo "installed to $dest:"
ls -la "$dest/kerf" "$dest/kerfd" "$dest/kerf-mcp"
if [ "$(uname -s)" = Darwin ] && [ -d "$apps_dir/Industrial Kerf.app" ]; then
    echo "editor: $apps_dir/Industrial Kerf.app"
elif [ -e "$dest/kerf-editor" ]; then
    echo "editor: $dest/kerf-editor"
fi
