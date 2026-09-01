#!/bin/sh
set -eu

# Runs the real cutd process test. The test allocates its own isolated project
# directory and tears it down, so this command is safe to rerun.
cargo test -p cut-runtime --test ipc_integration -- --nocapture
