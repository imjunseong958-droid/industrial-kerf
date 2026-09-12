---
name: install-cut-binaries
description: >-
  Builds and installs this Cut repo's local binaries (lcut, cutd, cut-mcp,
  cut-qt) into ~/.local/bin. Use in the cut repository when the user asks to
  빌드, 깔아, install binaries, reinstall lcut/cut-qt, or refresh after
  ffmpeg/playback/Qt bridge changes. Do not use in other repos.
---

# Install Cut local binaries

This skill is **this repository only**. Do not copy the paths or package set into another project.

## When to run

- User asks to build/install/reinstall `lcut`, `cutd`, `cut-mcp`, `cut-qt`, or "바이너리 깔아".
- Playback, `ffmpeg-next`, `cut-qt-bridge`, or `apps/cut-qt` changed and the editor must pick it up.
- `~/.local/bin/lcut` is missing or still launches an old Qt/bridge.

Do **not** `cargo install` from crates.io. Do **not** `brew install` Cut. Homebrew is only for Qt/FFmpeg/pkg-config/cmake.

## Default action

From the repo root, execute:

```bash
chmod +x scripts/install-cut-local.sh
./scripts/install-cut-local.sh all
```

That builds and writes:

| Binary | Build | Install |
|---|---|---|
| `lcut` | `cargo build --release -p cut-cli --bin lcut` | copy → `~/.local/bin/lcut` |
| `cutd` | `cargo build --release -p cut-runtime --bin cutd` | symlink → `~/.local/bin/cutd` |
| `cut-mcp` | `cargo build --release -p cut-mcp --bin cut-mcp` | copy → `~/.local/bin/cut-mcp` |
| `cut-qt` | `cargo build -p cut-qt-bridge` then cmake `apps/cut-qt` → `target/qt/cut-qt` | copy → `~/.local/bin/cut-qt` |

`cut-qt` links `target/debug/libcut_qt_bridge.dylib` (also present under `target/debug/deps/`). After a bridge rebuild, cmake-link `cut-qt` and recopy it.

Override the install dir with `CUT_LOCAL_BIN` if needed. Default is `$HOME/.local/bin`.

## Partial rebuilds

- **CLI/daemon/MCP only:** `./scripts/install-cut-local.sh cli`
- **Qt/playback/FFmpeg bindings only:** `./scripts/install-cut-local.sh qt`
- Playback or `ffmpeg-next` changes always need the `qt` path. `lcut` does not link libav.

## Host deps

pkg-config must see `libavcodec`, `libavformat`, `libavfilter`, `libswscale`, `libswresample`. On this Mac that is Homebrew FFmpeg (currently 9). Qt is Homebrew `qt`. Linker flags already live in `.cargo/config.toml`.

## After install

Confirm `which lcut` is `~/.local/bin/lcut` (PATH must include `~/.local/bin`). Do not commit unless the user asks. Do not push unless the user asks.
