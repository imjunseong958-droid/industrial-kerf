#!/usr/bin/env bash
# Install Cut's local editor/CLI binaries to ~/.local/bin, with the editor's
# own libraries (the Qt bridge and docking) in ~/.local/lib/cut. The install
# does not refer back to this checkout, so cleaning or moving it is safe.
# System libraries (Qt, FFmpeg) still come from their package manager.
set -euo pipefail

cut_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$cut_root"
dest="${CUT_LOCAL_BIN:-$HOME/.local/bin}"
lib_dir="${CUT_LOCAL_LIB:-$(dirname "$dest")/lib/cut}"
mkdir -p "$dest"

mode="${1:-all}"

build_cli() {
    cargo build --release -p cut-cli --bin lcut
    cargo build --release -p cut-runtime --bin cutd
    cargo build --release -p cut-mcp --bin cut-mcp
}

build_qt() {
    cargo build -p cut-qt-bridge
    if command -v brew >/dev/null 2>&1; then
        cmake -S apps/cut-qt -B target/qt -DCMAKE_PREFIX_PATH="$(brew --prefix qt)"
    else
        cmake -S apps/cut-qt -B target/qt
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
    for binary in lcut cutd cut-mcp; do
        place "$cut_root/target/release/$binary" "$dest/$binary"
    done
}

# Point the installed editor at the libraries copied beside it instead of
# the build tree, then re-sign what changed (macOS refuses to run binaries
# whose signature no longer matches).
relink_qt_macos() {
    local editor="$dest/cut-qt"
    local bridge_ref
    bridge_ref="$(otool -L "$editor" | awk '/libcut_qt_bridge/ {print $1; exit}')"
    place "$cut_root/target/debug/libcut_qt_bridge.dylib" "$lib_dir/libcut_qt_bridge.dylib"
    install_name_tool -id @rpath/libcut_qt_bridge.dylib "$lib_dir/libcut_qt_bridge.dylib"
    install_name_tool -change "$bridge_ref" @rpath/libcut_qt_bridge.dylib "$editor"
    local ref name
    for ref in $(otool -L "$editor" | awk '/@rpath\/libqtadvanceddocking/ {print $1}'); do
        name="${ref#@rpath/}"
        place "$(find "$cut_root/target/qt" -name "$name" -print -quit)" "$lib_dir/$name"
        codesign --force --sign - "$lib_dir/$name" 2>/dev/null
    done
    local rpath
    for rpath in $(otool -l "$editor" | awk '/LC_RPATH/ {getline; getline; print $2}'); do
        case "$rpath" in "$cut_root"/*) install_name_tool -delete_rpath "$rpath" "$editor" ;; esac
    done
    install_name_tool -add_rpath "$lib_dir" "$editor"
    codesign --force --sign - "$lib_dir/libcut_qt_bridge.dylib" "$editor" 2>/dev/null
}

relink_qt_linux() {
    local editor="$dest/cut-qt" library
    for library in "$cut_root/target/debug/libcut_qt_bridge.so" \
                   $(find "$cut_root/target/qt" -name 'libqtadvanceddocking*.so*' -type f); do
        place "$library" "$lib_dir/$(basename "$library")"
    done
    if command -v patchelf >/dev/null 2>&1; then
        patchelf --replace-needed "$cut_root/target/debug/libcut_qt_bridge.so" libcut_qt_bridge.so "$editor" 2>/dev/null || true
        patchelf --set-rpath "$lib_dir" "$editor"
    else
        echo "warning: patchelf not found; cut-qt still loads its libraries from $cut_root/target" >&2
    fi
}

install_qt() {
    mkdir -p "$lib_dir"
    place "$cut_root/target/qt/cut-qt" "$dest/cut-qt"
    case "$(uname -s)" in
        Darwin) relink_qt_macos ;;
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
ls -la "$dest/lcut" "$dest/cutd" "$dest/cut-mcp" "$dest/cut-qt"
