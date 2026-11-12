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
- a transcription runs on half the CPUs, four at most (whisper.cpp alone takes four whatever the machine has)
- decode hwaccel: VideoToolbox on macOS (not in a virtual machine, where it can stop answering), D3D11VA on Windows (`KERF_HWACCEL=none` disables; `KERF_HWACCEL=cuda` forces another backend)

`kerf-engine resources` prints the resolved budget. Other knobs: `KERF_FFMPEG_THREADS`,
`KERF_WHISPER_THREADS`, `KERF_EXTRACT_JOBS`, `KERF_PRESET`, `KERF_CRF`.

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
| `probe media-info` | codec, profile, pixel format, bit depth, colour tags, and the camera, lens, reel and timecode tags a file carries |
| `render --preset standard\|review\|prores_hq` | a program render; `review` is full-size H.264 at 12 Mb/s, `prores_hq` the ProRes 422 HQ master in QuickTime with 24-bit PCM |
| `loudness` | programme EBU R128 loudness (`loudness::measure_program`): integrated, LRA, true peak per channel, momentary/short-term series every 100 ms |
| `qc` | the picture QC pass (`qc::picture_qc`): Rec.709 gamut (BRNG), Y′ 64–940, flashing, black |

Loudness mixes the program's sound with the render graph and runs FFmpeg's
`ebur128` over it; levels with no signal are `-inf` in Rust and `null` in
JSON. Picture QC draws every frame with the compositor at 360 p, converts it
to 10-bit 4:2:2 Rec.709 limited range as a master is encoded, and measures it
with `signalstats` and `blackdetect`. The ProRes master uses
`prores_videotoolbox` on a Mac where it opens, otherwise `prores_ks` profile 3.

Transcription uses `KERF_WHISPER_BIN` (or `whisper-cli` on `PATH`) and
`KERF_WHISPER_MODEL`. It prepares mono 16 kHz WAV ranges and stores typed JSON
under `.kerf/cache/derived`; `listen` requires a start plus an end/duration and
is limited to two minutes. `KERF_WHISPER_CPP_VERSION` and
`KERF_WHISPER_MODEL_VERSION` can pin cache identity. Provider transcription is
not selected from API keys and is unavailable until an explicit upload/consent
adapter is implemented.
