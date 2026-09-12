#!/usr/bin/env bash
# Install Cut's local editor/CLI binaries to ~/.local/bin.
set -euo pipefail

cut_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$cut_root"
dest="${CUT_LOCAL_BIN:-$HOME/.local/bin}"
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

install_cli() {
    cp -f "$cut_root/target/release/lcut" "$dest/lcut"
    chmod 755 "$dest/lcut"
    ln -sfn "$cut_root/target/release/cutd" "$dest/cutd"
    cp -f "$cut_root/target/release/cut-mcp" "$dest/cut-mcp"
    chmod 755 "$dest/cut-mcp"
}

install_qt() {
    cp -f "$cut_root/target/qt/cut-qt" "$dest/cut-qt"
    chmod 755 "$dest/cut-qt"
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
ls -la "$dest/lcut" "$dest/cutd" "$dest/cut-mcp" "$dest/cut-qt"
