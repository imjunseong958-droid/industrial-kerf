---
name: run-kerf-editor
description: Launch and drive the real Kerf Qt editor (apps/kerf-qt) on a project, to see a UI or drawing change in its window rather than in the offscreen tests. Use when asked to open, run, or screenshot the editor, or to check the Inspector, the picture's overlays, or the Finish tray for real.
---

# Run the Kerf editor

The offscreen tests check strings and state, not what a window looks like at its real size, so
use this when the question is what a person sees. Written from a session on an M1 Mac with 8 GB.
On Linux without a display, see "Linux, offscreen" below: tried on Ubuntu 24.04 with Qt 6.4.2.

## Build

Each as its own command, with nothing heavy alongside on 8 GB:

```bash
export RUSTC_WRAPPER=
cargo build -q --release -p kerf-qt-bridge
cargo build -q --release -p kerf-runtime --bin kerfd
cmake -S apps/kerf-qt -B target/qt -DKERF_QT_BUILD_TESTS=ON -DKERF_BRIDGE_LIBRARY=$PWD/target/release/libkerf_qt_bridge.dylib
cmake --build target/qt --target kerf-qt --parallel
```

Rebuild the bridge after any change to the engine, or the editor runs the old drawing code.

## A project to open

A project is a folder with a `project.kerf` and its media. Make it in a temporary folder, so the
editor's edits to `project.kerf` touch nothing real. This one has clips in depth, both bevels, a
soft shadow, an aperture, a sampled path, and a title off the frame's edge:

```bash
P=$TMPDIR/kerf-run/project; mkdir -p $P/media
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

Run the bundle's binary directly, in the background and with the Bash sandbox off, since the sandbox
blocks both Metal and `open`. `KERFD_BIN` says which daemon to start:

```bash
KERFD_BIN=$PWD/target/release/kerfd ./target/qt/kerf-qt.app/Contents/MacOS/kerf-qt "$P" > "$TMPDIR/kerf-run/editor.log" 2>&1
```

It takes about six seconds to show the picture. A blank picture is a failed launch: read the log.

## Drive it

With the computer-use tools, the app's bundle identifier is `com.industrial-kerf.editor`. Look at
every screenshot. Launching proves little.

- **Media:** Window → Media (⌘1). It lies over the conversation. Double-click a clip there to insert
  it at the playhead; the title bar goes to "Edited just now", and `project.kerf` is rewritten a few
  seconds later, so read it back to check.
- **A path:** select its clip. The curve and its handles lie on the picture, and the handles drag.

Things that look like faults and are not:

- Hovering over the picture skims: the time follows the pointer across the picture, with a grey line
  on the timeline, unless masks or a path can be dragged.
- A click refused with "Could not verify which document this window is editing" comes from a tooltip's
  window. Move the pointer away, take a new screenshot, and click again.

## Linux, offscreen

With no display, the editor still draws its window to a file. Build as CI's `linux-editor` job does
(the debug bridge and `kerfd`, then CMake), with `FFMPEG_DIR` and `LD_LIBRARY_PATH` on the pinned
FFmpeg 9. Then, with `$P` a project as above and `$D` an empty folder for the run's settings, so the
user's recent projects are left alone:

```bash
QT_QPA_PLATFORM=offscreen KERFD_BIN=$PWD/target/debug/kerfd target/qt/kerf-qt "$P" \
  --settings-dir "$D" --window-size 1600x1000 --screenshot /tmp/editor.png --quit-after-ms 9000
pkill -f "debug/kerfd"
```

Without a project it captures the start window. This shows the layout, the conversation, the
timeline and a CPU-drawn picture. It cannot be clicked: for a state that needs a click, use a test's
gallery (`KERF_SCREENSHOT_DIR`), and say that no pointer was tried.

## Leave things as they were

Closing the editor does not stop its daemon, and opening a project adds it to the recent list:

```bash
pkill -f "target/qt/kerf-qt.app/Contents/MacOS/kerf-qt"
pkill -f "release/kerfd --dir .*kerf-run/project"
defaults read "com.industrial-kerf.Industrial Kerf" projects.recent
```

Take the temporary project off that list and leave every other entry as it is, by writing the same
list back without it: `defaults write "com.industrial-kerf.Industrial Kerf" projects.recent -array "<each other path>"`.
Then remove `$TMPDIR/kerf-run`.

## Say what was not checked

Name what the window could not show: an HDR display, sound when the scene has none, an export, any
other machine.
