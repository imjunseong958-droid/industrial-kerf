# Cut Qt editor

This is the native Qt6 desktop shell for Cut. It keeps the project state in
`cutd` through the small C ABI in `bridge.h`; the window does not maintain a
second editable copy of the timeline. The left media bin, viewer, timeline,
inspector, and right agent rail are all backed by the bridge event stream.

Build the Rust bridge first, then configure and build the app from the
repository root:

```sh
cargo build -p cut-qt-bridge
cmake -S apps/cut-qt -B target/qt
cmake --build target/qt
target/qt/cut-qt /path/to/project /path/to/cutd
```

The positional arguments may also be supplied as `CUT_PROJECT_DIR` and
`CUTD_BIN`. For a quick visual smoke run, add
`--screenshot /tmp/cut.png --quit-after-ms 900`.

On macOS the default bridge path is
`target/debug/libcut_qt_bridge.dylib`; on Linux pass
`-DCUT_BRIDGE_LIBRARY=$PWD/target/debug/libcut_qt_bridge.so` when configuring.
The project directory is created by the runtime when it does not already
exist. Importing media, timeline edits, undo/redo, proposal approval, preview
seeking, and export are sent as bounded asynchronous bridge commands.

The timeline supports click-to-seek, clip selection, drag-to-move, edge
trimming, split-at-playhead, and Delete. Inspector values commit on focus loss.
