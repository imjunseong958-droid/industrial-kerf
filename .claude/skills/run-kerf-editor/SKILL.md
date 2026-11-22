---
name: run-kerf-editor
description: Launch and drive the real Kerf editor (apps/kerf-editor, the Rust app drawn with GPUI) on a project, to see a UI or drawing change in its window rather than in tests. Use when asked to open, run, or screenshot the editor, or to check the Inspector, the picture's overlays, or the Finish tray for real.
---

# Run the Kerf editor

Tests check state, not what a window looks like at its real size, so use this when the question
is what a person sees. The editor is one Rust binary, `kerf-editor`, drawn with GPUI: Metal on
macOS, Vulkan on Linux. `--screenshot <file>` writes a PNG of its first window as the run quits
(after `--quit-after-ms`): read back from the X server on X11, by `screencapture` on macOS. On
Wayland and Windows it says on stderr that it cannot, so take the picture from outside there.

## Build

Each as its own command, with nothing heavy alongside on a small machine:

```bash
cargo build -q --manifest-path apps/kerf-editor/Cargo.toml
cargo build -q -p kerf-runtime --bin kerfd
```

Use `--release` for both to judge playback: the live preview draws its frames in `kerf-bridge`,
which the editor links, so a change to the engine needs the editor rebuilt, not only kerfd. On
Linux GPUI needs `libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libxcb1-dev
libx11-xcb-dev libfontconfig-dev libfreetype-dev libvulkan1` to build, and a Vulkan driver to
run: with no GPU, Mesa's lavapipe (`mesa-vulkan-drivers`). The FFmpeg it links is found as
kerfd's is (`FFMPEG_DIR`, with `LD_LIBRARY_PATH` on its `lib` on Linux).

## A project to open

A project is a folder with a `project.kerf` and its media. Make it in a temporary folder, so the
editor's edits to `project.kerf` touch nothing real. This one has clips in depth, both bevels, a
soft shadow, an aperture, a sampled path, and a title off the frame's edge:

```bash
P=${TMPDIR:-/tmp}/kerf-run/project; mkdir -p $P/media
ffmpeg -hide_banner -loglevel error -y -f lavfi -i testsrc2=s=3840x2160:r=24:d=4 -pix_fmt yuv420p $P/media/clip.mp4
cat > $P/project.kerf <<'KERF'
kerf 1
project name="Run"
media m1 path="media/clip.mp4" rate=24 dur=00:00:04:00 video=3840x2160
sequence s1 rate=24 size=1920x1080 audio=48000
  track V1 video
    clip footage media=m1 src=0f at=0f dur=96f
      set three_d=true z=0.4
  track V2 video
    clip block gen=solid@1 at=0f dur=96f
      set color=#e08030 three_d=true x=-0.3 y=0.18 z=-0.1 scale=0.26 depth=0.2 bevel=0.04 rotate_y=30 rotate_x=-20
  track V3 video
    clip word gen=title@1 at=0f dur=96f
      set text="Bevel" size=0.22 three_d=true z=-0.15 y=-0.15 depth=0.1 bevel=0.03 bevel_profile=round
  track V4 video
    clip walker gen=solid@1 at=0f dur=96f
      set color=#30a0e0 three_d=true anchor_x=0.25 rotate_y=30 scale_x=0.18 scale_y=0.1 orient=true path="-0.35,-0.25 -0.05,0.25 0.3,-0.2" along={0f:0 96f:1}
  track V5 video
    clip edge gen=title@1 at=0f dur=96f
      set text="Off the edge" size=0.12 x=0.45 y=0.4
  track V6 video
    clip lamp gen=light@1 at=0f dur=96f
      set kind=directional x=-0.4 y=0.3 z=1 shadow=1 size=4
  track V7 video
    clip lens gen=camera@1 at=0f dur=96f
      set aperture=0.03
KERF
```

## Launch

`kerf-editor [project-dir] [kerfd-binary]`; `KERF_PROJECT_DIR` and `KERFD_BIN` do the same.
Without a project it shows the launcher. Point `KERF_EDITOR_PREFS` at a scratch file, so the run
does not add the project to the person's recent list in `~/.kerf/editor.json`:

```bash
KERF_EDITOR_PREFS=${TMPDIR:-/tmp}/kerf-run/editor.json KERFD_BIN=$PWD/target/debug/kerfd \
  target/debug/kerf-editor "$P" > "${TMPDIR:-/tmp}/kerf-run/editor.log" 2>&1 &
```

`--quit-after-ms <ms>` makes it quit on its own after that long. On macOS run it in the
background with the Bash sandbox off, since the sandbox blocks Metal. It takes a few seconds to
show the picture. A black or empty window is a failed launch: read the log (`RUST_LOG=info`
says which GPU adapter it took).

## Linux, with no display

Run it on a virtual X server. Xvfb needs the Composite extension, or the editor's Vulkan frames
come out black. `--screenshot` takes the picture as the run quits; give the picture time to
settle (a 3D scene drawn on the CPU took over 9 seconds here) and the same `--window-size` each
time:

```bash
xvfb-run -a -s "-screen 0 1600x1000x24 +extension Composite" \
    env KERF_EDITOR_PREFS=${TMPDIR:-/tmp}/kerf-run/editor.json KERFD_BIN=$PWD/target/debug/kerfd \
    target/debug/kerf-editor "$P" --window-size 1600x1000 --quit-after-ms 20000 \
    --screenshot /tmp/kerf.png
```

Then look at `/tmp/kerf.png` with the Read tool. If Vulkan finds no driver, or takes one that
draws nothing, point it at lavapipe:
`VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json`. The run needs `xvfb`, `imagemagick`,
`xdotool` and `mesa-vulkan-drivers`.

To reach a state that needs a click, start it in the background and click with `xdotool` before
it quits; the picture then shows the state after the click. With no window manager a quick
`click` only moves the hover, so press and release apart:

```bash
xvfb-run -a -s "-screen 0 1600x1000x24 +extension Composite" sh -c '
    KERF_EDITOR_PREFS=${TMPDIR:-/tmp}/kerf-run/editor.json KERFD_BIN=$PWD/target/debug/kerfd \
        target/debug/kerf-editor "'"$P"'" --window-size 1600x1000 --quit-after-ms 20000 \
        --screenshot /tmp/kerf.png & E=$!
    sleep 8; W=$(xdotool search --pid $E | tail -1)
    xdotool mousemove --window "$W" 1310 20; sleep 1
    xdotool mousedown 1; sleep 0.2; xdotool mouseup 1
    wait $E'
```

`xdotool type` types into the window the same way. Settings and the Finish tray open inside the
window, so the picture has them; a second window (another project's) is not in it: take it with ImageMagick's
`import -window <id>`, the id from `xdotool search --pid`. GPUI draws a frame only after an
input event when there is no window manager, so move the pointer before an `import`, or its
picture is black or stale.

## Drive it on a desktop

With the computer-use tools on macOS, the bundled app's identifier is
`com.industrial-kerf.editor` (`scripts/bundle-kerf-macos.sh` makes `Industrial Kerf.app` around a
built `kerf-editor`); a plain `target/debug/kerf-editor` runs too, without the bundle. Look at
every screenshot. Launching proves little.

- **Media:** place a clip from the media at the playhead; `project.kerf` is rewritten a few
  seconds later, so read it back to check.
- **A path:** select its clip. The curve and its handles lie on the picture, and the handles drag.

## Leave things as they were

Closing the editor does not stop its daemon:

```bash
pkill -f "target/debug/kerf-editor"
pkill -f "kerfd --dir .*kerf-run/project"
```

Then remove `${TMPDIR:-/tmp}/kerf-run`. If the run went without `KERF_EDITOR_PREFS`, take the
temporary project out of `recent` in `~/.kerf/editor.json` and leave every other entry as it is.

## Say what was not checked

Name what the window could not show: an HDR display, sound when the scene has none, an export, a
real GPU when it ran on lavapipe, any other machine.
