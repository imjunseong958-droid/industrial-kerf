#!/usr/bin/env bash
set -euo pipefail
kerf_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$kerf_root"
cargo build -p kerf-qt-bridge -p kerf-runtime --lib --bin kerfd
if command -v brew >/dev/null 2>&1; then
    cmake -S apps/kerf-qt -B target/qt -DCMAKE_PREFIX_PATH="$(brew --prefix qt)"
else
    cmake -S apps/kerf-qt -B target/qt
fi
cmake --build target/qt --parallel
exec "$kerf_root/target/qt/kerf-qt" "${1:-${KERF_PROJECT_DIR:-$kerf_root/.kerf/native-project}}" "${KERFD_BIN:-$kerf_root/target/debug/kerfd}"
