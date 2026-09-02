# ADR 0002: Local transcription backend

Status: accepted for Phase 6
Date: 2026-09-02

## Context

Transcription is an expensive media observation.  It must remain bounded,
repeatable, cancellable, and private.  Cut already has FFmpeg/ffprobe sidecars
and a project-local derived-artifact cache, but it must not add a second media
runtime or silently upload source media to a provider.

## Decision

- The initial backend is an optional local `whisper.cpp` CLI sidecar.  Cut
  resolves `CUT_WHISPER_BIN` or `whisper-cli`/`whisper-cpp`/`main` on `PATH` and
  requires a local GGML model from `CUT_WHISPER_MODEL` or the request.
- Provider processing is not implemented in Phase 6.  A provider backend is
  only represented for an explicit request and returns a capability-unavailable
  result; API keys never select it implicitly.
- Any local source that FFmpeg/ffprobe can decode to an audio stream is
  supported; Cut first extracts the requested range to mono 16 kHz PCM WAV.
  Transcription requests are limited to 15 minutes; `listen` requests are
  explicitly bounded to 2 minutes and require a start plus an end or duration.
- The sidecar is invoked with JSON-full output and progress reporting.  The
  adapter normalizes segment/token/word timestamps into a typed word/cue
  schema, offsetting them back into source time.
- Cancellation is an in-process token checked by the worker.  It kills and
  waits for the exact FFmpeg or whisper.cpp child, so a cancelled operation
  cannot publish a complete cache manifest.
- Cache entries use the existing `.cut/cache/derived` store.  The key includes
  source file identity, selected range, backend/version, executable identity,
  model file identity/version, language, and WAV normalization options.
- Audio and transcript JSON remain regenerable derived artifacts.  They are
  returned as paths and typed metadata; they are never written to canonical
  project state.  Creating captions from a transcript remains a separate,
  explicit typed project command.
- Sidecar stderr is kept to a bounded diagnostic tail.  Transcript text, audio
  bytes, provider payloads, and credentials are not copied into UI logs or
  canonical state.

## Consequences

The feature has no network dependency and gives a clear setup error when the
optional sidecar or model is absent.  The first run can be slower because it
prepares a WAV and loads the model, while repeated requests with the same
source and configuration reuse the cache.  Packaging each target platform
must decide separately whether to bundle whisper.cpp and a model.  Phase 6
does not bundle either one; a future bundle would add a native binary and a
large model per macOS, Windows, and Linux architecture, plus platform signing
and update costs.

The CLI contract follows whisper.cpp's `whisper-cli` output flags (`-ojf`,
`-of`, `-l`, and `-pp`) but treats the JSON parser as a compatibility boundary
because sidecar builds can expose segment, token, or word timestamp shapes.
