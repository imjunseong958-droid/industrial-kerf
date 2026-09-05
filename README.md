# cut

Cut is a native, agentic video editor. `lcut` opens the Qt6 editor by default;
pass `--terminal` for the project-scoped terminal agent. `cut-mcp` is the
project-scoped MCP server.

## Architecture

`cutd` is the only owner of project state, revisions, history, idempotency
receipts, proposals, and render jobs. The native editor, terminal agent, and
MCP server are adapters over the same project-scoped IPC boundary.

```text
Qt6 editor ──────────────────────────┐
Rust agent and CLI ──────────────────┼─> cutd ─> typed domain ─> durable store
Rust MCP server ─────────────────────┘               └────────> cut-engine
```

FFmpeg, ffprobe, yt-dlp, and ONNX remain controlled native sidecars. Editing
playbooks are data in `skills/*/SKILL.md` and are loaded by `cut-skills`.

## Build and run

```bash
cargo build --release -p cut-runtime --bin cutd
cargo build -p cut-qt-bridge
cmake -S apps/cut-qt -B target/qt
cmake --build target/qt
cargo build --release -p cut-cli --bin lcut
cargo build --release -p cut-mcp --bin cut-mcp
```

Open the Qt editor for a project:

```bash
CUT_QT_BIN="$PWD/target/qt/cut-qt" target/release/lcut \
  --dir ./my-project --cutd "$PWD/target/release/cutd"
```

Run the terminal workflow:

```bash
target/release/lcut --terminal --dir ./my-project video.mp4
target/release/lcut --terminal --dir ./my-project -p "trim the first clip to two seconds"
target/release/lcut --terminal --dir ./my-project --skills
target/release/lcut --terminal --dir ./my-project --generate-docs
target/release/lcut --terminal --dir ./my-project --mcp
target/release/lcut --terminal --dir ./my-project --mcp-tools -p "use the connected research tool"
```

In editor mode, `lcut` imports a local media path or an HTTP(S) URL before
opening the native window. The editor's agent rail uses the same provider
configuration as the terminal agent. `--cutd PATH` selects an explicit
daemon; the `CUTD_BIN` environment variable does the same.

`--mcp` lists the native catalog and any servers configured in
`~/.cut/mcp.json` or `<project>/.cut/mcp.json`. `--mcp-tools` (or
`CUT_ENABLE_MCP=1`) attaches those inbound stdio servers to the terminal agent
through `mcp_search` and `mcp_call`; configured tool allowlists are enforced.

The Qt bridge, daemon, decoder, agent proposal, and restart smoke test are
available at:

```bash
cargo build -p cut-qt-bridge -p cut-runtime --lib --bin cutd
python3 scripts/test-cut-qt-bridge.py
```

## Providers

| Provider | Key | Transports |
|---|---|---|
| Anthropic | `ANTHROPIC_API_KEY` | Messages SSE |
| OpenAI | `OPENAI_API_KEY` | Chat Completions or Responses |

Provider selection uses `--provider`, then the configured key. Models and
OpenAI-compatible endpoints can be set with CLI flags or `~/.cut/config.json`.
Keep credentials outside project directories and never commit them.

```bash
export OPENAI_API_KEY=...
export OPENAI_BASE_URL=https://openrouter.ai/api/v1
target/release/lcut --provider openai --model anthropic/claude-sonnet-4 \
  --dir ./my-project -p "make a short vertical preview"
```

Responses mode and reasoning effort are selected with `--api-mode responses`
and `--reasoning-effort low|medium|high|...`.

## MCP

`cut-mcp` speaks JSON-RPC MCP over stdin/stdout. Diagnostics go to stderr and
all project mutations go through `cutd`.

```bash
target/release/cut-mcp --dir ./my-project
target/release/cut-mcp --dir ./my-project --full
```

Use `CUTD_BIN` when `cutd` is not on `PATH`:

```json
{
  "mcpServers": {
    "cut": {
      "command": "/absolute/path/to/target/release/cut-mcp",
      "args": ["--dir", "/absolute/path/to/my-project"]
    }
  }
}
```

The core profile contains the stable editing loop:

| Tool | Purpose |
|---|---|
| `project.outline` | Read the structural sequence, track, clip, caption, effect, and marker hierarchy |
| `timeline.query` | Inspect tracks, clips, source ranges, and placement |
| `timeline.check` | Check source availability, timeline coverage, compiler findings, and capability mismatches |
| `timeline.capture` | Capture a deterministic offline timeline range as a frame or filmstrip |
| `timeline.apply` | Apply one canonical v2 command or atomic batch |
| `timeline.undo` / `timeline.redo` | Move durable history |
| `media.probe` | Run info, silence, scenes, exact frames, filmstrips, waveform, transcription, or selected-range listening |
| `media.fetch` / `media.import` | Fetch or import media through `cutd` |
| `render.preview` / `render.start` / `render.result` | Run and retrieve renders |
| `skills.read` | Load a playbook body by exact name |

Local transcription is optional and never uploads media implicitly. Install a
`whisper.cpp` `whisper-cli` binary and configure a local GGML model before using
`media.probe` with `action: "transcribe"` or `action: "listen"`:

```bash
export CUT_WHISPER_BIN=/absolute/path/to/whisper-cli
export CUT_WHISPER_MODEL=/absolute/path/to/ggml-base.bin
target/debug/cut-engine probe listen input.mp4 --start 12 --duration 30 --project-dir ./my-project
```

`listen` requires an explicit bounded range and is limited to two minutes;
`transcribe` accepts up to fifteen minutes per request. Both actions return
typed cues and word timestamps from `.cut/cache/derived`, while the listening
action also returns the prepared local WAV path. Set
`CUT_WHISPER_CPP_VERSION` and `CUT_WHISPER_MODEL_VERSION` when packaging or
pinning sidecar/model versions. A configured provider backend is rejected until
an explicit upload-capable adapter and UI consent flow exist.

The `--full` profile adds project inspection and validation, timeline diff,
render polling/cancellation, and the optional vision capability.

`lcut --generate-docs` writes deterministic, machine-owned protocol, tool,
skill, and current-project summary files under
`.cut/docs/generated/`. Keep editorial guidance in
`.cut/docs/conventions.md`; the generator never overwrites it or a root
`AGENTS.md`. If a root pointer is useful, pass the explicit
`--add-agents-pointer` opt-in; it appends an idempotent pointer without
replacing existing instructions. The outline is a projection of the `cutd`
snapshot and cannot mutate project state.

## Interchange

`cut-interchange` validates the project and invokes the configured native or
external adapter through a small JSON bridge. OpenTimelineIO conversion uses
the bundled `crates/cut-interchange/python/otio_bridge.py` and requires an
environment with `opentimelineio` plus the requested adapter installed.

## Verification

Focused Rust tests cover the durable runtime, real FFmpeg playback, 4K/VFR/
long-GOP media, audio clock mapping, captions, provider adapters, MCP stdio,
skills, fetch, interchange, CLI wiring, and protocol conformance fixtures.

```bash
cargo test --workspace
cargo clippy -p cut-agent -p cut-mcp -p cut-cli -p cut-qt-bridge \
  -p cut-fetcher -p cut-interchange -p cut-skills -p cut-playback \
  -p cut-runtime --all-targets --no-deps -- -D warnings
```

The Qt release gate also includes a manual window check: open a project, edit
it, restart it, enter Korean text, play/seek media, verify captions/mute, and
accept or reject an agent proposal.
