---
name: install-kerf-binaries
description: >-
  Builds and installs this Kerf repo's local binaries (kerf, kerfd, kerf-mcp,
  kerf-qt) into ~/.local/bin. Use in the Industrial Kerf repository when the user asks to
  빌드, 깔아, install binaries, reinstall kerf/kerf-qt, or refresh after
  ffmpeg/playback/Qt bridge changes. Do not use in other repos.
---

# Install Kerf local binaries

This skill is **this repository only**. Do not copy the paths or package set into another project.

## When to run

- User asks to build/install/reinstall `kerf`, `kerfd`, `kerf-mcp`, `kerf-qt`, or "바이너리 깔아".
- Playback, `ffmpeg-next`, `kerf-qt-bridge`, or `apps/kerf-qt` changed and the editor must pick it up.
- `~/.local/bin/kerf` is missing or still launches an old Qt/bridge.

Do **not** `cargo install` from crates.io. Do **not** `brew install` Kerf. Homebrew is only for Qt/FFmpeg/pkg-config/cmake.

## Default action

From the repo root, execute:

```bash
chmod +x scripts/install-kerf-local.sh
./scripts/install-kerf-local.sh all
```

That builds and writes:

| Binary | Build | Install |
|---|---|---|
| `kerf` | `cargo build --release -p kerf-cli --bin kerf` | copy → `~/.local/bin/kerf` |
| `kerfd` | `cargo build --release -p kerf-runtime --bin kerfd` | symlink → `~/.local/bin/kerfd` |
| `kerf-mcp` | `cargo build --release -p kerf-mcp --bin kerf-mcp` | copy → `~/.local/bin/kerf-mcp` |
| `kerf-qt` | `cargo build -p kerf-qt-bridge` then cmake `apps/kerf-qt` → `target/qt/kerf-qt` | copy → `~/.local/bin/kerf-qt` |

`kerf-qt` links `target/debug/libkerf_qt_bridge.dylib` (also present under `target/debug/deps/`). After a bridge rebuild, cmake-link `kerf-qt` and recopy it.

Override the install dir with `KERF_LOCAL_BIN` if needed. Default is `$HOME/.local/bin`.

## Partial rebuilds

- **CLI/daemon/MCP only:** `./scripts/install-kerf-local.sh cli`
- **Qt/playback/FFmpeg bindings only:** `./scripts/install-kerf-local.sh qt`
- Playback or `ffmpeg-next` changes always need the `qt` path. `kerf` does not link libav.

## Host deps

pkg-config must see `libavcodec`, `libavformat`, `libavfilter`, `libswscale`, `libswresample`. On this Mac that is Homebrew FFmpeg (currently 9). Qt is Homebrew `qt`. Linker flags already live in `.cargo/config.toml`.

## After install

Confirm `which kerf` is `~/.local/bin/kerf` (PATH must include `~/.local/bin`). Do not commit unless the user asks. Do not push unless the user asks.
