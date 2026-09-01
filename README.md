# cut

Cut is a native, agentic video editor. The packaged desktop binary is `cut`;
the terminal workflow is `lcut` and the project-scoped MCP server is
`cut-mcp`.

## Architecture

`cutd` is the only owner of project state, revisions, history, idempotency
receipts, proposals, and render jobs. The native editor, terminal agent, and
MCP server are adapters over the same project-scoped IPC boundary.

```text
native editor (winit + egui + wgpu) ─┐
Rust agent and CLI ──────────────────┼─> cutd ─> typed domain ─> durable store
Rust MCP server ─────────────────────┘               └────────> cut-engine
```

FFmpeg, ffprobe, yt-dlp, and ONNX remain controlled native sidecars. Editing
playbooks are data in `skills/*/SKILL.md` and are loaded by `cut-skills`.

## Build and run

```bash
cargo build --release -p cut-runtime --bin cutd
cargo build --release -p cut-app --bin cut
cargo build --release -p cut-cli --bin lcut
cargo build --release -p cut-mcp --bin cut-mcp
```

Open the native editor for a project:

```bash
target/release/cut --dir ./my-project --cutd "$PWD/target/release/cutd"
```

Run the terminal workflow:

```bash
target/release/lcut --dir ./my-project video.mp4
target/release/lcut --dir ./my-project -p "trim the first clip to two seconds"
target/release/lcut --dir ./my-project --skills
target/release/lcut --dir ./my-project --mcp
target/release/lcut --dir ./my-project --mcp-tools -p "use the connected research tool"
```

`lcut` imports a local media path or an HTTP(S) URL, then uses the Rust agent
when a prompt or API key is configured. Without a key it prints the current
project snapshot and exits. `--cutd PATH` selects an explicit daemon; the
`CUTD_BIN` environment variable does the same.

`--mcp` lists the native catalog and any servers configured in
`~/.cut/mcp.json` or `<project>/.cut/mcp.json`. `--mcp-tools` (or
`CUT_ENABLE_MCP=1`) attaches those inbound stdio servers to the terminal agent
through `mcp_search` and `mcp_call`; configured tool allowlists are enforced.

The native command surface and daemon restart smoke test are available at:

```bash
scripts/smoke-cut-app.sh
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
| `timeline.query` | Inspect tracks, clips, source ranges, and placement |
| `timeline.apply` | Apply one canonical v2 command or atomic batch |
| `timeline.undo` / `timeline.redo` | Move durable history |
| `media.probe` | Run info, silence, scenes, or frame analysis |
| `media.fetch` / `media.import` | Fetch or import media through `cutd` |
| `render.preview` / `render.start` / `render.result` | Run and retrieve renders |
| `skills.read` | Load a playbook body by exact name |

The `--full` profile adds project inspection and validation, timeline diff,
render polling/cancellation, and the optional vision capability.

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
cargo clippy -p cut-agent -p cut-mcp -p cut-cli -p cut-app \
  -p cut-fetcher -p cut-interchange -p cut-skills -p cut-playback \
  -p cut-runtime --all-targets --no-deps -- -D warnings
```

The native release gate also includes a manual window check: open a project,
edit it, restart it, enter Korean text, traverse controls with Tab, inspect the
AccessKit tree, play/seek media, verify captions/mute/fullscreen, and accept or
reject an agent proposal.
