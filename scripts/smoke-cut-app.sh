#!/usr/bin/env bash
set -euo pipefail

script_root=$(cd "$(dirname "$0")/.." && pwd)
cd "$script_root"

cargo build --release -p cut-runtime --bin cutd
cargo build --release -p cut-app --bin cut

project_dir=$(mktemp -d)
daemon_pid=''

cleanup() {
  if [ -n "$daemon_pid" ]; then
    kill "$daemon_pid" 2>/dev/null || true
    wait "$daemon_pid" 2>/dev/null || true
  fi
}
trap cleanup EXIT

start_daemon() {
  target/release/cutd --dir "$project_dir" >/dev/null 2>&1 &
  daemon_pid=$!
}

start_daemon
target/release/cut --smoke --dir "$project_dir" --cutd "$script_root/target/release/cutd" \
  >"$project_dir/first-smoke.json"

kill "$daemon_pid"
wait "$daemon_pid" 2>/dev/null || true
daemon_pid=''

start_daemon
target/release/cut --smoke --dir "$project_dir" --cutd "$script_root/target/release/cutd" \
  >"$project_dir/restart-smoke.json"
rg -q '"initialRevision": 3' "$project_dir/restart-smoke.json"

echo "native cut-app command and restart smoke passed: $project_dir"
