# cut-engine

Native media engine for Cut. `cutd`, the agent harness, and the Qt bridge call
it in-process as the `cut_engine` library; the `cut-engine` binary exposes the
same operations for debugging and scripting.

```
cargo test -p cut-engine
cargo build --release -p cut-engine
cargo build --profile maxperf -p cut-engine   # fat LTO, slowest link, fastest binary
```

Release builds use thin LTO, `codegen-units = 1`, `panic = abort`, and mimalloc.
The timeline compiler and overlay primitives live in the dependency-light
`cut-core` crate. `cut-engine` re-exports that API while keeping ffmpeg and
media dependencies in the media engine.

Default resource limits:

- CPU budget from `available_parallelism` (override with `CUT_CPUS`)
- ffmpeg thread caps so decode does not oversubscribe
- hardware encode by default when VideoToolbox/NVENC/QSV/AMF is listed (`--software` or `CUT_ENCODER=software` to force libx264; `--prefer-hardware` to force a hardware attempt)
- libx264 `veryfast`/`faster` on small machines instead of `medium`
- frame extraction parallelism is capped
- decode hwaccel: VideoToolbox on macOS, D3D11VA on Windows (`CUT_HWACCEL=none` disables; `CUT_HWACCEL=cuda` forces another backend)

`cut-engine resources` prints the resolved budget. Other knobs: `CUT_FFMPEG_THREADS`,
`CUT_EXTRACT_JOBS`, `CUT_PRESET`, `CUT_CRF`.

ffmpeg/ffprobe paths, encoder listings, and hwaccel flags are resolved once per
process. Multi-frame `probe frames` seeks in bounded parallel. Exact frame,
filmstrip, waveform, and timeline-capture artifacts use the project-local
`.cut/cache/derived` cache and publish a manifest only after generation
completes. Compile regexes are cached.

Commands:

| Command | Role |
|---|---|
| `probe info/silence/scenes/frames/frame/filmstrip/waveform` | ffprobe/ffmpeg analysis, exact frames, contact sheets, and waveform derivatives |
| `probe transcribe/listen` | Optional local whisper.cpp transcription and explicitly bounded listening observations |
| `encoders` / `select-encoder` | hardware encoder discovery (`select-encoder` stays opt-in) |
| `resources` | laptop CPU/encode budget |
| `proxy-key` / `proxy-args` / `proxy-run` | deterministic proxy cache keys, proxy ffmpeg argv, and proxy transcodes |
| `plan` / `crop` | Region keyframe planning and aspect-aware crop planning from observed boxes |

Transcription uses `CUT_WHISPER_BIN` (or `whisper-cli` on `PATH`) and
`CUT_WHISPER_MODEL`. It prepares mono 16 kHz WAV ranges and stores typed JSON
under `.cut/cache/derived`; `listen` requires a start plus an end/duration and
is limited to two minutes. `CUT_WHISPER_CPP_VERSION` and
`CUT_WHISPER_MODEL_VERSION` can pin cache identity. Provider transcription is
not selected from API keys and is unavailable until an explicit upload/consent
adapter is implemented.
