# cut-engine

Native media engine for Cut. TypeScript packages (`@cut/media`, `@cut/vision`)
always invoke this binary (`target/release/cut-engine`, `target/debug/cut-engine`,
or `CUT_ENGINE_BIN`). There is no TypeScript engine fallback.

```
cargo test -p cut-engine
cargo build --release -p cut-engine
cargo build --profile maxperf -p cut-engine   # fat LTO, slowest link, fastest binary
cargo build --features onnx -p cut-engine   # opt-in native YOLO inference
```

Release builds use thin LTO, `codegen-units = 1`, `panic = abort`, and mimalloc.
The default development build excludes ONNX Runtime so editing and media-only
work does not compile the native inference stack. Add `--features onnx` for a
tracking build.

The timeline compiler and overlay primitives live in the dependency-light
`cut-core` crate. `cut-engine` re-exports that API while keeping ffmpeg and
vision dependencies in the media engine.

Default resource limits:

- CPU budget from `available_parallelism` (override with `CUT_CPUS`)
- ffmpeg/ONNX thread caps so decode and infer do not oversubscribe
- hardware encode by default when VideoToolbox/NVENC/QSV/AMF is listed (`--software` or `CUT_ENCODER=software` to force libx264; `--prefer-hardware` to force a hardware attempt)
- libx264 `veryfast`/`faster` on small machines instead of `medium`
- YOLO tracking samples at 8 fps (5 fps on 2-core) and downscales in ffmpeg before RGB
- frame extraction parallelism is capped
- decode hwaccel: VideoToolbox on macOS, D3D11VA on Windows (`CUT_HWACCEL=none` disables; `CUT_HWACCEL=cuda` forces another backend)

`cut-engine resources` prints the resolved budget. Other knobs: `CUT_FFMPEG_THREADS`,
`CUT_DECODE_THREADS`, `CUT_ONNX_THREADS`, `CUT_EXTRACT_JOBS`, `CUT_TRACK_FPS`,
`CUT_PRESET`, `CUT_CRF`.

ffmpeg/ffprobe paths, encoder listings, and hwaccel flags are resolved once per
process. Multi-frame `probe frames` seeks in bounded parallel. Compile regexes are
cached. YOLO letterbox writes NCHW in-place (no per-frame tensor clone). Frame
decode from ffmpeg uses a 1 MiB buffered pipe. JSONL tracking output is written
directly rather than via nested maps.

Commands:

| Command | Role |
|---|---|
| `probe info/silence/scenes/frames` | ffprobe/ffmpeg analysis |
| `encoders` / `select-encoder` | hardware encoder discovery (`select-encoder` stays opt-in) |
| `resources` | laptop CPU/encode/tracking budget |
| `compile-v1` / `compile-v2` / `duration-v1` / `duration-v2` / `render-v2` | filtergraph, duration, ffmpeg argv (`--prefer-hardware` / `--software`) |
| `overlay` | path/drawtext escape, atempo, captions, subtitles |
| `proxy-key` / `proxy-args` | deterministic proxy cache keys |
| `normalize` / `plan` / `crop` / `track` | YOLO JSONL normalize, ROI planning, ByteTrack; tracking loads `.onnx` (or a sibling `.onnx` next to a `.pt` checkpoint) |

YOLO inference is letterbox → decode → NMS → ByteTrack in-process via ONNX Runtime.
`.pt` files are not executed; export with `yolo export model=custom.pt format=onnx`.
Python Ultralytics remains a training/export tool only.
