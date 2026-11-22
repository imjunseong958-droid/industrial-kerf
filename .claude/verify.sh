#!/bin/sh
# industrial-kerf checks: the gate README.md's Verification names, which the
# Rust runtime CI also runs on Linux. Keep the three in sync.
set -e
cargo fmt --all --check
cargo fmt --manifest-path apps/kerf-editor/Cargo.toml --all --check
cargo clippy --workspace --all-targets --no-deps -- -D warnings
cargo test --workspace
cargo clippy --manifest-path apps/kerf-editor/Cargo.toml --workspace --all-targets -- -D warnings
cargo test --manifest-path apps/kerf-editor/Cargo.toml --workspace
