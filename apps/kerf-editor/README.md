# Industrial Kerf editor

The native desktop editor for Kerf: one Rust program, `kerf-editor`, drawn with
[GPUI](https://www.gpui.rs), the UI framework Zed is built on. It keeps the project in
`kerfd` through `kerf-bridge` (`crates/surfaces/kerf-bridge`), which it links; the
window keeps no second editable copy of the project, and draws what the bridge's events
leave. It draws with Metal on macOS, DirectX 11 on Windows, and Vulkan on Linux, on X11
or Wayland.

## Build

From the repository root, with kerfd beside it:

```sh
cargo build --manifest-path apps/kerf-editor/Cargo.toml
cargo build -p kerf-runtime --bin kerfd
```

The editor is a Cargo workspace of its own, not a member of the repository's: GPUI turns
on serde_json's `preserve_order`, and in one workspace that feature would reach every
program built beside the editor, kerfd among them, whose journal is checksummed over JSON
it writes again. So it is built, tested and linted with `--manifest-path apps/kerf-editor/Cargo.toml`
(`cargo test --manifest-path apps/kerf-editor/Cargo.toml`, `cargo clippy --manifest-path apps/kerf-editor/Cargo.toml --all-targets`), into the
repository's `target` (`.cargo/config.toml`), and its version is the repository's
(`tests/editor/version.rs`). It links FFmpeg through the bridge, found as
kerfd finds it (`FFMPEG_DIR`, or `pkg-config`). Build it with `--release` to judge
playback: the live preview draws its frames in the bridge.

What GPUI needs to build:

- **Linux**: xkbcommon, xcb, Wayland, fontconfig and freetype, and the Vulkan loader. On
  Debian and Ubuntu:

  ```sh
  sudo apt install libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libxcb1-dev \
      libx11-xcb-dev libfontconfig-dev libfreetype-dev libvulkan1 mesa-vulkan-drivers
  ```

  At run time it loads libxkbcommon, libxcb, libwayland-client and the Vulkan loader
  from the system, and needs a Vulkan driver: Mesa's (`mesa-vulkan-drivers`), or the GPU
  vendor's. With no GPU, Mesa's lavapipe draws it in software.
- **macOS**: Xcode, whose `metal` compiles GPUI's shaders.
- **Windows**: Visual Studio's C++ tools, for the linker and the resource compiler that
  puts Kerf's icon in `kerf-editor.exe` (`assets/icon/kerf-editor.rc`).

## Run

```sh
target/debug/kerf-editor [project-dir] [kerfd-binary]
```

The project and kerfd may come from `KERF_PROJECT_DIR` and `KERFD_BIN` instead. Without
a project the editor opens the one last worked on, or with none to go back to, its
project launcher. A `kerf://<project>/rev<N>@HH:MM:SS:FF` link in place of the project
opens the recent project it names at that moment (`src/links.rs`); macOS hands links to
the running editor. `kerf` opens it the same way, finding it
beside itself, in the installed app on macOS, or in `target/debug` and `target/release`
(`KERF_EDITOR_BIN` names another), and passes it kerfd through `KERFD_BIN`.

- `--quit-after-ms <ms>` quits after that long, for a smoke run; the package checks start
  the editor so and need it to exit cleanly. A smoke run (one that quits on its own, or
  asks for `--screenshot`) opens no project it was not given, and says nothing on its own
  at launch.
- `--settings-dir <dir>` keeps the editor's settings in that folder (`editor.json`), and
  keys out of the system's keychain; `KERF_EDITOR_PREFS` names the file itself.
- `--window-size WxH` (as `1600x1000`) opens each window of the run at that size rather
  than edge to edge.
- `--render-preview` renders the project's preview once it is open, saying nothing else
  at launch.
- `--screenshot <file>` writes a PNG of the first window as the run quits (after
  `--quit-after-ms`, else half a second) on X11 and macOS; on Wayland and Windows it says
  it cannot, and the picture is taken from outside (below).
- `--graph <media>` opens that shot's graph once the project has it, with
  `--graph-node <id>` chosen in it.
- `--serve-mcp --dir <project>` opens no window: it serves Kerf's tools over MCP on stdin
  and stdout, as `kerf-mcp` does. Claude Code, as the editor's chat provider, is given the
  editor to run this way where no `kerf-mcp` is found (`kerf-bridge`'s `claude_code.rs`).

Off macOS, an editor started for `kerf://` links alone hands them to the editor already
open, through a link inbox (a socket in the runtime folder, else a port named in the
settings folder), and ends. The first time it runs, the editor takes up the settings the
editor before 0.1.5 (Qt) kept (its provider, models, faces, keymap, recent projects and
each project's master output), once, so an update loses none of them.

Where `--screenshot` cannot (Wayland, Windows), or to catch the window mid-run, a picture
of it is taken from outside.
On Linux with no display, on a virtual X server, which needs the Composite extension or
the Vulkan frames come out black, with ImageMagick's `import` and `xdotool`:

```sh
xvfb-run -a -s "-screen 0 1600x1000x24 +extension Composite" sh -c '
    target/debug/kerf-editor /path/to/project & P=$!
    sleep 3; xdotool mousemove 800 500; sleep 6
    import -window "$(xdotool search --pid $P | tail -1)" /tmp/kerf.png
    kill $P'
```

With no window manager, GPUI draws its first frame on the first input event, so the
pointer is moved over the window before the picture is taken.

`KERF_REDUCE_MOTION=1` turns motion off, as Settings ▸ Appearance does
([MOTION_UI.md](MOTION_UI.md)), and `RUST_LOG=kerf_editor=debug` logs every event the
bridge sends.

If Vulkan finds no driver there, point it at lavapipe:
`VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json`. On macOS, `screencapture -l`
takes a window by its id.

## Assets

`assets/fonts` holds the faces the interface is set in, each with its licence beside it
(`licenses/README.md`, Fonts); the packages carry the licences in their `fonts` folder.
`assets/icon` holds Kerf's icon, drawn once in `industrial-kerf.svg` and rendered by
`tools/kerf-icon` to the PNG, `.ico` and `.icns` files beside it
([assets/icon/README.md](assets/icon/README.md)).

## Packaging

`packaging/` holds what the packages carry beside the programs; the scripts in
`scripts/` build the editor with `cargo build --release --manifest-path apps/kerf-editor/Cargo.toml` and put it with
`kerfd`, `kerf`, `kerf-mcp`, `ffmpeg` and `ffprobe`:

- **Windows** (`scripts/package-kerf-windows.ps1`): a zip with `kerf-editor.exe`, the
  FFmpeg DLLs and the Visual C++ runtime, and the same folder as an installer
  (`packaging/windows/kerf.iss`, Inno Setup) that registers `kerf://` links.
- **macOS** (`scripts/package-kerf-macos.sh`): `Industrial Kerf.app`, laid out by
  `scripts/bundle-kerf-macos.sh` from `packaging/macos/Info.plist`, with FFmpeg and the
  Homebrew libraries it loads copied into `Contents/Frameworks`.
- **Linux** (`scripts/package-kerf-linux.sh`, `-deb.sh`, `-rpm.sh`): a folder with the
  programs in `bin/` and FFmpeg in `lib/`, taking from the system only what
  `packaging/linux/system-libraries.txt` lists, packed as a `.tar.xz`, a `.deb` and an
  `.rpm`; `packaging/linux/install-desktop-entry.sh` registers the folder's editor for
  `kerf://` links, and `industrial-kerf.desktop` is the entry the `.deb` and `.rpm`
  install.

Each `THIRD-PARTY.txt` says what its package carries and on what terms. The
`scripts/check-kerf-*` scripts run what was packed with nothing else on `PATH` and fail
when it loads anything it should carry from outside itself.
