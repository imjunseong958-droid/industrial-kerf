#!/usr/bin/env bash
# Install Kerf's local editor/CLI binaries to ~/.local/bin, with the editor's
# own libraries (the Qt bridge and docking) in ~/.local/lib/kerf; on macOS
# the editor is installed as an app too, ~/Applications/Industrial Kerf.app.
# The install does not refer back to this checkout, so cleaning or moving it
# is safe.
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

# Point the installed editor at the libraries copied beside it instead of
# the build tree, then re-sign what changed (macOS refuses to run binaries
# whose signature no longer matches).
relink_qt_macos() {
    local editor="${1:-$dest/kerf-qt}"
    local bridge_ref
    bridge_ref="$(otool -L "$editor" | awk '/libkerf_qt_bridge/ {print $1; exit}')"
    place "$bridge_dir/libkerf_qt_bridge.dylib" "$lib_dir/libkerf_qt_bridge.dylib"
    install_name_tool -id @rpath/libkerf_qt_bridge.dylib "$lib_dir/libkerf_qt_bridge.dylib"
    install_name_tool -change "$bridge_ref" @rpath/libkerf_qt_bridge.dylib "$editor"
    local ref name
    for ref in $(otool -L "$editor" | awk '/@rpath\/libqtadvanceddocking/ {print $1}'); do
        name="${ref#@rpath/}"
        place "$(find "$kerf_root/target/qt" -name "$name" -print -quit)" "$lib_dir/$name"
        codesign --force --sign - "$lib_dir/$name" 2>/dev/null
    done
    local rpath
    for rpath in $(otool -l "$editor" | awk '/LC_RPATH/ {getline; getline; print $2}'); do
        case "$rpath" in "$kerf_root"/*) install_name_tool -delete_rpath "$rpath" "$editor" ;; esac
    done
    install_name_tool -add_rpath "$lib_dir" "$editor"
    codesign --force --sign - "$lib_dir/libkerf_qt_bridge.dylib" "$editor" 2>/dev/null
}

relink_qt_linux() {
    local editor="$dest/kerf-qt" library
    for library in "$bridge_dir/libkerf_qt_bridge.so" \
                   $(find "$kerf_root/target/qt" -name 'libqtadvanceddocking*.so*' -type f); do
        place "$library" "$lib_dir/$(basename "$library")"
    done
    if command -v patchelf >/dev/null 2>&1; then
        patchelf --replace-needed "$bridge_dir/libkerf_qt_bridge.so" libkerf_qt_bridge.so "$editor" 2>/dev/null || true
        patchelf --set-rpath "$lib_dir" "$editor"
    else
        echo "warning: patchelf not found; kerf-qt still loads its libraries from $kerf_root/target" >&2
    fi
}

# The editor as an app, for Finder, the Dock and Spotlight: the build's
# bundle in place of an older install's (never copied over it), its
# executable pointed at the same libraries, and kerfd inside it, where the
# editor looks first (an app opened from Finder has no shell PATH).
install_qt_bundle() {
    local app="$apps_dir/Industrial Kerf.app"
    local kerfd="$dest/kerfd"
    [ -x "$kerfd" ] || kerfd="$kerf_root/target/release/kerfd"
    mkdir -p "$apps_dir"
    rm -rf "$app"
    cp -R "$kerf_root/target/qt/kerf-qt.app" "$app"
    relink_qt_macos "$app/Contents/MacOS/kerf-qt"
    if [ -x "$kerfd" ]; then
        place "$kerfd" "$app/Contents/MacOS/kerfd"
        codesign --force --sign - "$app/Contents/MacOS/kerfd" 2>/dev/null
    else
        echo "warning: kerfd is not built; the app looks for kerfd on its PATH (run '$0 all')" >&2
    fi
    codesign --force --sign - "$app" 2>/dev/null
}

install_qt() {
    mkdir -p "$lib_dir"
    case "$(uname -s)" in
        # The executable out of the application bundle the build makes.
        Darwin) place "$kerf_root/target/qt/kerf-qt.app/Contents/MacOS/kerf-qt" "$dest/kerf-qt" ;;
        *) place "$kerf_root/target/qt/kerf-qt" "$dest/kerf-qt" ;;
    esac
    case "$(uname -s)" in
        Darwin)
            relink_qt_macos
            install_qt_bundle
            ;;
        Linux) relink_qt_linux ;;
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

echo "installed to $dest (editor libraries in $lib_dir):"
ls -la "$dest/kerf" "$dest/kerfd" "$dest/kerf-mcp" "$dest/kerf-qt"
if [ "$(uname -s)" = Darwin ] && [ -d "$apps_dir/Industrial Kerf.app" ]; then
    echo "app: $apps_dir/Industrial Kerf.app"
fi
