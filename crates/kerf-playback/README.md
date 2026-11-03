# `kerf-playback`

`kerf-playback` is the native Unit 5 playback boundary. It accepts a typed
`kerf_model::Project`, builds an immutable timeline plan, and runs in-process
libav video decoding and audio work away from the editor event loop. Export
still uses the ffmpeg CLI in `kerf-engine`.

The public surface is intentionally small:

- `PlaybackSession` owns a worker-backed session.
- `PlaybackHandle` provides bounded open, play, pause, seek, volume, mute,
  solo, and shutdown commands. A solo (`set_solo`) hears only some audio
  tracks of the program sequence, for monitoring; the project never changes.
- `PlaybackSession::levels` is a polled reading of the program as it is
  heard: L/R sample peak and RMS in dBFS over the 1024-frame blocks of the
  mix heard since the last reading (so no peak is skipped between polls),
  and BS.1770-4 momentary (400 ms) and short-term (3 s) loudness in LUFS
  (`src/loudness.rs`).
- `PlaybackClock::dropped` counts the frames the presenter skipped as late
  since the last open or seek.
- `PlaybackEvent` delivers opened metadata, state and clock updates, decoded
  RGBA frames, end-of-media, and errors.
- `PlaybackTimeline` exposes tested timeline-to-source mapping and caption
  lookup for adapters and focused tests.

Video frames are decoded in-process with libavformat/libavcodec/libavfilter
to packed RGBA and delivered by `crates/kerf-qt-bridge` to the Qt6 viewer.
Audio uses cpal when a default output device is available and otherwise uses a
monotonic clock fallback, still mixing (and metering) the program in real
time, level with the clock. Both command and decode/event paths are bounded
so media work does not block the UI thread.

Build requires system FFmpeg development libraries (`pkg-config` must see
`libavcodec`, `libavformat`, `libavfilter`, `libswscale`, `libswresample`).
Set `FFMPEG_DIR` if those `.pc` files are not on the default search path.

Focused verification:

```bash
cargo test -p kerf-playback --all-targets -- --nocapture
cargo test -p kerf-playback --no-default-features --all-targets -- --nocapture
cargo clippy -p kerf-playback --all-targets -- -D warnings
cargo clippy -p kerf-playback --no-default-features --all-targets -- -D warnings
```

The FFmpeg integration test creates a short real media file and verifies RGBA
frame delivery. `tests/release_gate.rs` covers real 4K, variable-frame-rate,
long-GOP, captions, mute, fullscreen, and audio synchronization behavior.
