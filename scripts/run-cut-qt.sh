#!/usr/bin/env bash
set -euo pipefail
cut_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$cut_root"
cargo build -p cut-qt-bridge -p cut-runtime --lib --bin cutd
if command -v brew >/dev/null 2>&1; then
    cmake -S apps/cut-qt -B target/qt -DCMAKE_PREFIX_PATH="$(brew --prefix qt)"
else
    cmake -S apps/cut-qt -B target/qt
fi
cmake --build target/qt --parallel
exec "$cut_root/target/qt/cut-qt" "${1:-${CUT_PROJECT_DIR:-$cut_root/.cut/native-project}}" "${CUTD_BIN:-$cut_root/target/debug/cutd}"
