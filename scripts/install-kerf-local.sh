#!/usr/bin/env bash
# Install Kerf's command-line tools (kerf, kerfd, kerf-mcp) to ~/.local/bin,
# and the editor: on macOS as an app, ~/Applications/Industrial Kerf.app,
# which carries its own libraries and kerfd; elsewhere as ~/.local/bin/kerf-qt
# with its libraries (the Qt bridge and docking) in ~/.local/lib/kerf. The
# install does not refer back to this checkout, so cleaning or moving it is
# safe.
# System libraries (Qt, FFmpeg) still come from their package manager.
# The bridge is built optimized: the editor's live preview and its exports
# draw frames inside it.
set -euo pipefail

kerf_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$kerf_root"
dest="${KERF_LOCAL_BIN:-$HOME/.local/bin}"
lib_dir="${KERF_LOCAL_LIB:-$(dirname "$dest")/lib/kerf}"
apps_dir="${KERF_LOCAL_APPS:-$HOME/Applications}"
bridge_dir="$kerf_root/target/release"
mkdir -p "$dest"

mode="${1:-all}"

build_cli() {
    cargo build --release -p kerf-cli --bin kerf
    cargo build --release -p kerf-runtime --bin kerfd
    cargo build --release -p kerf-mcp --bin kerf-mcp
}

build_qt() {
    cargo build --release -p kerf-qt-bridge
    local bridge
    case "$(uname -s)" in
        Darwin) bridge="$bridge_dir/libkerf_qt_bridge.dylib" ;;
        *) bridge="$bridge_dir/libkerf_qt_bridge.so" ;;
    esac
    if command -v brew >/dev/null 2>&1; then
        cmake -S apps/kerf-qt -B target/qt -DCMAKE_PREFIX_PATH="$(brew --prefix qt)" \
            -DKERF_BRIDGE_LIBRARY="$bridge"
    else
        cmake -S apps/kerf-qt -B target/qt -DKERF_BRIDGE_LIBRARY="$bridge"
    fi
    cmake --build target/qt --parallel
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

relink_qt_linux() {
    local editor="$dest/kerf-qt"
    place "$bridge_dir/libkerf_qt_bridge.so" "$lib_dir/libkerf_qt_bridge.so"
    if command -v patchelf >/dev/null 2>&1; then
        patchelf --replace-needed "$bridge_dir/libkerf_qt_bridge.so" libkerf_qt_bridge.so "$editor" 2>/dev/null || true
        patchelf --set-rpath "$lib_dir" "$editor"
    else
        echo "warning: patchelf not found; kerf-qt still loads its libraries from $kerf_root/target" >&2
    fi
}

# The editor as an app, for Finder, the Dock and Spotlight, and its only
# install on macOS (kerf opens it too): the build's bundle in place of an
# older install's (never copied over it), carrying the Qt bridge in
# Contents/Frameworks and kerfd beside its executable, where the editor
# looks first (an app opened from Finder has no shell PATH). It is signed
# again inside out, as macOS refuses to run code
# whose signature no longer matches.
install_qt_macos() {
    local app="$apps_dir/Industrial Kerf.app"
    local editor="$app/Contents/MacOS/kerf-qt"
    local frameworks="$app/Contents/Frameworks"
    local kerfd="$dest/kerfd"
    [ -x "$kerfd" ] || kerfd="$kerf_root/target/release/kerfd"
    mkdir -p "$apps_dir"
    rm -rf "$app"
    cp -R "$kerf_root/target/qt/kerf-qt.app" "$app"
    mkdir -p "$frameworks"
    local bridge_ref
    bridge_ref="$(otool -L "$editor" | awk '/libkerf_qt_bridge/ {print $1; exit}')"
    place "$bridge_dir/libkerf_qt_bridge.dylib" "$frameworks/libkerf_qt_bridge.dylib"
    install_name_tool -id @rpath/libkerf_qt_bridge.dylib "$frameworks/libkerf_qt_bridge.dylib"
    install_name_tool -change "$bridge_ref" @rpath/libkerf_qt_bridge.dylib "$editor"
    local rpath
    for rpath in $(otool -l "$editor" | awk '/LC_RPATH/ {getline; getline; print $2}'); do
        case "$rpath" in "$kerf_root"/*) install_name_tool -delete_rpath "$rpath" "$editor" ;; esac
    done
    install_name_tool -add_rpath @executable_path/../Frameworks "$editor"
    if [ -x "$kerfd" ]; then
        place "$kerfd" "$app/Contents/MacOS/kerfd"
        codesign --force --sign - "$app/Contents/MacOS/kerfd" 2>/dev/null
    else
        echo "warning: kerfd is not built; the app looks for kerfd on its PATH (run '$0 all')" >&2
    fi
    codesign --force --sign - "$frameworks"/*.dylib 2>/dev/null
    codesign --force --sign - "$app" 2>/dev/null
    # What an older install left in ~/.local: kerf would open that stale
    # editor before the app.
    rm -f "$dest/kerf-qt" "$lib_dir/libkerf_qt_bridge.dylib" "$lib_dir"/libqtadvanceddocking*.dylib
    rmdir "$lib_dir" 2>/dev/null || true
}

install_qt() {
    case "$(uname -s)" in
        Darwin) install_qt_macos ;;
        *)
            mkdir -p "$lib_dir"
            place "$kerf_root/target/qt/kerf-qt" "$dest/kerf-qt"
            relink_qt_linux
            ;;
    esac
}

case "$mode" in
    all)
        build_cli
        build_qt
        install_cli
        install_qt
        ;;
    cli)
        build_cli
        install_cli
        ;;
    qt)
        build_qt
        install_qt
        ;;
    *)
        echo "usage: $0 [all|cli|qt]" >&2
        exit 2
        ;;
esac

echo "installed to $dest:"
ls -la "$dest/kerf" "$dest/kerfd" "$dest/kerf-mcp"
if [ "$(uname -s)" = Darwin ] && [ -d "$apps_dir/Industrial Kerf.app" ]; then
    echo "editor: $apps_dir/Industrial Kerf.app"
elif [ -e "$dest/kerf-qt" ]; then
    echo "editor: $dest/kerf-qt (libraries in $lib_dir)"
fi
