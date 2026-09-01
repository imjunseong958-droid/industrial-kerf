# `cut-app`

`cut-app` is the native desktop composition root. It owns the winit window,
egui view composition, app configuration, icon input, and `cutd` sidecar
discovery. It owns no canonical project state: the `AppRuntime` adapter sends
every mutation to the project-scoped daemon with the displayed revision.

Build and run the native shell with:

```bash
cargo build --release -p cut-runtime --bin cutd
cargo build --release -p cut-app --bin cut
CUT_PROJECT_DIR="$PWD/.cut/native-project" \
  CUTD_BIN="$PWD/target/release/cutd" \
  target/release/cut
```

The real command surface can be exercised without a display:

```bash
scripts/smoke-cut-app.sh
```

The smoke path opens a project through `cutd`, adds a video track, performs
undo and redo, restarts the daemon, and verifies that the revision survives.

The viewer delegates media control to `cut-playback`. It polls worker events on
the winit redraw path, uploads the latest RGBA frame to an egui wgpu texture,
and keeps timeline state, captions, volume, mute, and fullscreen actions in
the app adapter.

The native release gate includes a manual window workflow: open a project, edit
it, restart `cut`, and confirm that the same revision and track list return.
Korean IME composition, keyboard tab navigation, focus restoration, and native
AccessKit tree/action delivery passed that workflow. The shell enables
AccessKit and routes winit IME and keyboard events through `egui-winit`.
