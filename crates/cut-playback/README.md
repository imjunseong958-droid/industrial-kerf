# `cut-playback`

`cut-playback` is the native Unit 5 playback boundary. It accepts a typed
`cut-domain::Project`, builds an immutable timeline plan, and runs in-process
libav video decoding and audio work away from the editor event loop. Export
still uses the ffmpeg CLI in `cut-engine`.

The public surface is intentionally small:

- `PlaybackSession` owns a worker-backed session.
- `PlaybackHandle` provides bounded open, play, pause, seek, volume, mute, and
  shutdown commands.
- `PlaybackEvent` delivers opened metadata, state and clock updates, decoded
  RGBA frames, end-of-media, and errors.
- `PlaybackTimeline` exposes tested timeline-to-source mapping and caption
  lookup for adapters and focused tests.

Video frames are decoded in-process with libavformat/libavcodec/libavfilter
to packed RGBA and delivered by `crates/cut-qt-bridge` to the Qt6 viewer.
Audio uses cpal when a default output device is available and otherwise uses a
monotonic clock fallback. Both command and decode/event paths are bounded so
media work does not block the UI thread.

Build requires system FFmpeg development libraries (`pkg-config` must see
`libavcodec`, `libavformat`, `libavfilter`, `libswscale`, `libswresample`).
Set `FFMPEG_DIR` if those `.pc` files are not on the default search path.

Focused verification:

```bash
cargo test -p cut-playback --all-targets -- --nocapture
cargo test -p cut-playback --no-default-features --all-targets -- --nocapture
cargo clippy -p cut-playback --all-targets -- -D warnings
cargo clippy -p cut-playback --no-default-features --all-targets -- -D warnings
```

The FFmpeg integration test creates a short real media file and verifies RGBA
frame delivery. `tests/release_gate.rs` covers real 4K, variable-frame-rate,
long-GOP, captions, mute, fullscreen, and audio synchronization behavior. The
manual open, edit, restart, IME, keyboard navigation, focus, and Qt workflow is
recorded in `docs/RUST_MIGRATION_HANDOFF.md`.
