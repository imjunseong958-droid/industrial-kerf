#!/usr/bin/env bash
# Build kerfd, the bridge, and the Qt editor, then open the editor: on the
# project given as the argument or in KERF_PROJECT_DIR, else on its project
# launcher.
set -euo pipefail
project="${1:-${KERF_PROJECT_DIR:-}}"
case "$project" in
    "" | /*) ;;
    *) project="$PWD/$project" ;;
esac
kerf_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$kerf_root"
cargo build -p kerf-qt-bridge -p kerf-runtime --lib --bin kerfd
if command -v brew >/dev/null 2>&1; then
    cmake -S apps/kerf-qt -B target/qt -DCMAKE_PREFIX_PATH="$(brew --prefix qt)"
else
    cmake -S apps/kerf-qt -B target/qt
fi
cmake --build target/qt --parallel
# kerfd goes through the environment, so without a project it is never read
# as one.
export KERFD_BIN="${KERFD_BIN:-$kerf_root/target/debug/kerfd}"
# On macOS the editor is built as an application bundle.
editor="$kerf_root/target/qt/kerf-qt"
if [ "$(uname -s)" = Darwin ]; then
    editor="$kerf_root/target/qt/kerf-qt.app/Contents/MacOS/kerf-qt"
fi
if [ -n "$project" ]; then
    exec "$editor" "$project"
fi
unset KERF_PROJECT_DIR
exec "$editor"
