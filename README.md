# cut

Cut is a native, agentic video editor. `lcut` opens the Qt6 editor by default;
pass `--terminal` for the project-scoped terminal agent. `cut-mcp` is the
project-scoped MCP server and primary external-agent integration surface.
External Claude, Cursor, and Codex-like clients connect through `cut-mcp`
using their own agent configuration; they do not need a provider API key
configured in Cut.

## Architecture

`cutd` is the only owner of project state, revisions, history, idempotency
receipts, proposals, and render jobs. The native editor, terminal agent, and
MCP server are adapters over the same project-scoped IPC boundary.

The canonical project is the `project.cut` text document
([the Cut language](docs/cut-language.md)). On first open, `cutd` imports an
existing v1 or v2 `project.cut.json` once and leaves it untouched. Edits are
edit scripts or typed operations; agent proposals carry their script and the
diff of the document they would produce.

```text
Qt6 editor ──────────────────────────┐
Rust agent and CLI ──────────────────┼─> cutd ─> cut-model ─> project.cut
Rust MCP server ─────────────────────┘     │
                                           └─> .cut document ─> render compiler,
                                                                playback, previews
```

Renders, playback, and previews read the model. A render job and a proposal
preview carry the `.cut` document with media paths resolved against the
project, and the render compiler lowers its first sequence to one FFmpeg
graph with exact times: picture tracks stack in order, sound comes from
audio tracks, transitions play across their cuts using the clips' handles,
nested sequences composite on their own canvas, and effects, transitions,
and generators lower from their catalog definitions. A feature without a
lowering is reported with the clip it is on. Native playback plays the same
sequence as cuts and asks for a rendered preview whenever it would show
something different. Editor snapshots carry the document and a timeline
view of it; the editor sends its edits back as typed operations.

FFmpeg, ffprobe, and yt-dlp remain controlled native sidecars. Editing
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
`CUT_ENABLE_MCP=1`) attaches those inbound stdio servers through `mcp_search`,
`mcp_describe`, and `mcp_call`; configured tool allowlists are enforced.
The Qt editor's Settings tab can enable the same attachment for its Chat,
edit the project config, or import a Cursor config. `CUT_ENABLE_MCP=1` remains
available as an environment fallback. Qt remains a presentation layer.

## Agents

Agents edit the same `.cut` document people do (see
[the Cut language](docs/cut-language.md)). Their context is the language
reference, generated from the feature catalog, and the current document,
annotated with each clip's end and source out. The built-in agent reads the
document again before every model request, so it always sees the current
revision and the state of its proposal. There is no tool discovery; three
tools cover the work:

| Tool | Purpose |
|---|---|
| `look` | No arguments: the annotated document and proposal state. `media`: that media's index — where it is used, scene cuts, pauses, loudest moments, sound onsets, and tempo, and with `transcript: true` what is said (`words: true` times each word). `sheet: N`: a contact sheet of N frames over `from`–`to`, of `media` or the sequence, as one image. `at`: up to eight full-size frames — source frames of `media`, or program frames of the sequence. `transcript: true` without `media`: what the sequence says as it plays. `sequence` picks a sequence other than the first. `proposal`: the same, as a pending proposal would make the project |
| `edit` | Run an edit script. It becomes a proposal, returned with the document diff, check findings, and a before/after preview: the frames either side of each cut it made or moved, and the middle of a clip it changed in place. `extend` runs the script on top of a pending proposal and makes the two one proposal; `revise` replaces it |
| `render` | Render the sequence (or another `sequence`), or `from`–`to`, as a draft or full-quality file — of the project, or of a pending `proposal`; `job` reads progress, `cancel` stops it |

An edit's check findings include what an editor would catch, as warnings:
black stretches with nothing on screen, picture that leaves the frame's
edges black, flash frames shorter than three frames, and sound cut inside a
word (known from transcripts of that media already made; nothing is
transcribed for the check).

Times are timecode (`01:00:04:12`), frames (`96f`), or seconds (`4.5s`):
media positions with `media`, sequence positions otherwise. The project
changes only when a person accepts a proposal.

When media is imported, `cutd` indexes it in the background — one FFmpeg
pass for scene cuts, pauses, and a loudness envelope from which loudest
moments, onsets, and tempo are found — and makes a proxy at most 960 pixels
wide for pictures wider than that. Draft renders and frames up to that width
use the proxy when it is ready; full renders and program frames use the
original. Indexes and proxies are cached in `.cut/cache/derived`.

Transcripts come from local `whisper.cpp` (`CUT_WHISPER_BIN` and
`CUT_WHISPER_MODEL`, described below); media is never uploaded. Scene cuts
and pauses are cached with the media's other derivatives in
`.cut/cache/derived`.

Skills hold editorial guidance only — what makes a good short, how a meme
lands — not tool procedures. Bundled skills are compiled into `cut-skills`;
user (`~/.cut/skills`) and project (`.cut/skills`) skills override them by
name. Every skill's guidance is part of the agent's instructions.

The built-in agent retains its original transcript but constructs a bounded
view for each provider request. It prioritizes the current user request,
the latest tool exchange, and earlier user requirements. Older evidence and
images are replaced by explicit `session.read` references; a recent result
whose text is too long keeps its frames and previews. That tool can
retrieve a message, a tool result's JSON pointer, or one archived image.
When earlier requirements cannot fit, the view identifies their archive
indices and tells the agent to read them before making assumptions. This is
deterministic context reduction, not an LLM-generated semantic summary.

`CUT_AGENT_INPUT_BUDGET` controls the input allowance (default 196608 units).
The allowance conservatively charges text by UTF-8 bytes plus overhead and
reserves 4096 units per image; it is not an exact vendor token count. Choose
an allowance below the selected model's context window, leaving room for
output and reasoning. A context-limit response causes one retry with a
smaller view, without re-executing tools. A user request or fixed catalog that
cannot fit produces an explicit error instead of silent truncation.

Inbound MCP transport limits each message to 4 MiB. On Unix, it times out
after 30 seconds (`CUT_MCP_TIMEOUT_MS` overrides it) and running calls observe
the agent's cancellation flag. A cancelled external operation
may have already taken effect; verify its state before retrying it.

The Qt bridge, daemon, decoder, agent proposal, and restart smoke test are
available at:

```bash
cargo build -p cut-qt-bridge -p cut-runtime --lib --bin cutd
python3 scripts/test-cut-qt-bridge.py
```

## Optional embedded-agent providers

Provider APIs are optional and only power Cut's embedded agent rail (and its
terminal agent workflow). They are not required for external agents using
`cut-mcp` or for native editing.

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

Generate a generic MCP client configuration, then copy the JSON into your
external client's MCP settings:

```bash
target/release/cut-mcp --dir ./my-project --print-config
```

`--print-config` prints pretty JSON with the absolute path of the running
`cut-mcp` executable and an absolute project path. Relative project paths are
resolved against the current working directory; the project need not exist.
This mode exits without starting `cutd` or changing the project.
`CUT_PROJECT_DIR` supplies the default project path when `--dir` is omitted.

To run the stdio server directly:

```bash
target/release/cut-mcp --dir ./my-project
```

Keep `cutd` beside `cut-mcp`, put it on `PATH`, or set `CUTD_BIN` in the client
process environment. The generated configuration has this shape:

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

`cut-mcp` serves the agent tools above — `look`, `edit`, and `render`. The
server instructions carry the language reference and editorial guidance;
`look` with no arguments, or the resource `cut://document`, returns the
annotated document, and `cut://reference` the reference. Frames and previews
arrive as MCP image content, as JPEG. An `edit` from an external client is a proposal
like any other: the project and revision stay unchanged until a person
accepts it in the editor. `CUT_MCP_NAMESPACE` prefixes the tool names (for
example `cut.look`).

Local transcription is optional and never uploads media. Install a
`whisper.cpp` `whisper-cli` binary and configure a local GGML model before
using `look` with `transcript: true`:

```bash
export CUT_WHISPER_BIN=/absolute/path/to/whisper-cli
export CUT_WHISPER_MODEL=/absolute/path/to/ggml-base.bin
target/debug/cut-engine probe listen input.mp4 --start 12 --duration 30 --project-dir ./my-project
```

Set `CUT_WHISPER_CPP_VERSION` and `CUT_WHISPER_MODEL_VERSION` when packaging
or pinning sidecar/model versions.

`lcut --generate-docs` writes deterministic, machine-owned protocol, tool,
skill, and current-project summary files under
`.cut/docs/generated/`. Keep editorial guidance in
`.cut/docs/conventions.md`; the generator never overwrites it or a root
`AGENTS.md`. If a root pointer is useful, pass the explicit
`--add-agents-pointer` opt-in; it appends an idempotent pointer without
replacing existing instructions. The project summary lists the sequences
and media of the current document and cannot change project state.

## Interchange

`cut-interchange` converts the program sequence of a project to a
frame-exact timeline (positions in frames of the sequence, media times in
frames of the media) and invokes the configured native or external adapter
through a small JSON bridge, reporting what the conversion leaves out.
OpenTimelineIO conversion uses the bundled
`crates/cut-interchange/python/otio_bridge.py` and requires an environment
with `opentimelineio` plus the requested adapter installed.

## Verification

Focused Rust tests cover the durable runtime, real FFmpeg playback, 4K/VFR/
long-GOP media, audio clock mapping, captions, provider adapters, MCP stdio,
skills, fetch, interchange, CLI wiring, and protocol conformance fixtures.

```bash
cargo test --workspace
cargo clippy --workspace --all-targets --no-deps -- -D warnings
```

The Qt release gate also includes a manual window check: open a project, edit
it, restart it, enter Korean text, play/seek media, verify captions/mute, and
accept or reject an agent proposal.
