# kerf-engine

Native media engine for Kerf. `kerfd`, the agent harness, and the Qt bridge call
it in-process as the `kerf_engine` library; the `kerf-engine` binary exposes the
same operations for debugging and scripting.

```
cargo test -p kerf-engine
cargo build --release -p kerf-engine
cargo build --profile maxperf -p kerf-engine   # fat LTO, slowest link, fastest binary
```

Release builds use thin LTO, `codegen-units = 1`, `panic = abort`, and mimalloc.
The timeline compiler and overlay primitives live in the dependency-light
`kerf-core` crate. `kerf-engine` re-exports that API while keeping ffmpeg and
media dependencies in the media engine.

Default resource limits:

- CPU budget from `available_parallelism` (override with `KERF_CPUS`)
- ffmpeg thread caps so decode does not oversubscribe
- hardware encode by default when VideoToolbox/NVENC/QSV/AMF is listed (`--software` or `KERF_ENCODER=software` to force libx264; `--prefer-hardware` to force a hardware attempt)
- libx264 `veryfast`/`faster` on small machines instead of `medium`
- frame extraction parallelism is capped
- decode hwaccel: VideoToolbox on macOS, D3D11VA on Windows (`KERF_HWACCEL=none` disables; `KERF_HWACCEL=cuda` forces another backend)

`kerf-engine resources` prints the resolved budget. Other knobs: `KERF_FFMPEG_THREADS`,
`KERF_EXTRACT_JOBS`, `KERF_PRESET`, `KERF_CRF`.

ffmpeg/ffprobe paths, encoder listings, and hwaccel flags are resolved once per
process. Multi-frame `probe frames` seeks in bounded parallel. Exact frame,
filmstrip, waveform, and timeline-capture artifacts use the project-local
`.kerf/cache/derived` cache and publish a manifest only after generation
completes. Compile regexes are cached.

Commands:

| Command | Role |
|---|---|
| `probe info/silence/scenes/frames/frame/filmstrip/waveform` | ffprobe/ffmpeg analysis, exact frames, contact sheets, and waveform derivatives |
| `probe transcribe/listen` | Optional local whisper.cpp transcription and explicitly bounded listening observations |
| `encoders` / `select-encoder` | hardware encoder discovery (`select-encoder` stays opt-in) |
| `resources` | laptop CPU/encode budget |
| `proxy-key` / `proxy-args` / `proxy-run` | deterministic proxy cache keys, proxy ffmpeg argv, and proxy transcodes |

Transcription uses `KERF_WHISPER_BIN` (or `whisper-cli` on `PATH`) and
`KERF_WHISPER_MODEL`. It prepares mono 16 kHz WAV ranges and stores typed JSON
under `.kerf/cache/derived`; `listen` requires a start plus an end/duration and
is limited to two minutes. `KERF_WHISPER_CPP_VERSION` and
`KERF_WHISPER_MODEL_VERSION` can pin cache identity. Provider transcription is
not selected from API keys and is unavailable until an explicit upload/consent
adapter is implemented.
