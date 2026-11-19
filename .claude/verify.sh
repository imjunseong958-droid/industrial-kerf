#!/bin/sh
# industrial-kerf checks: the gate README.md's Verification names, which the
# Rust runtime CI also runs on Linux. Keep the three in sync.
set -e
cargo clippy --workspace --all-targets --no-deps -- -D warnings
cargo test --workspace
