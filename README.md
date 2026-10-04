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
existing `project.cut.json` once and leaves it untouched.

```text
Qt6 editor ──────────────────────────┐
Rust agent and CLI ──────────────────┼─> cutd ─> typed domain ─> durable store
Rust MCP server ─────────────────────┘               └────────> cut-engine
```

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

## Agent discovery and context

Use `tools.search` for compact native tool descriptions, then `tools.describe`
for the selected schema. For edits, pass `name: "timeline.apply"` and a
`commandType` such as `trim_clip` or `update_clip` to retrieve one command's
fields and example. Without `commandType`, the response lists the available
command types. The built-in agent advertises a fixed set of bootstrap tools
and executes other discovered native operations through `tools.call`, keeping
new feature schemas out of every initial prompt. External MCP catalogs also
keep detailed command schemas behind `tools.describe`.
Atomic edits accept up to 256 command entries and 16 nested
batch levels.

Bundled playbooks are compiled into `cut-skills`; they do not depend on the
working directory or the build machine's source tree. User and project skills
override bundled names. The initial prompt includes at most 16 names with
short descriptions. Discover additional skills with paginated `skills.list`
and load one body with `skills.read`.

`timeline.query` accepts `start`, `end`, `trackId`, `clipId`, `offset`, and
`limit`. Time ranges are half-open. `project.outline` has a node budget;
expand a returned node's `path` with `nodePath`, `childOffset`, and `childLimit`.
Follow the returned cursors and recheck the revision when continuing a query.
External MCP results larger than 16 KiB become immutable `results.read`
handles. That tool reads a JSON pointer or UTF-8 page without repeating the
operation. Evidence is cached in `.cut/cache/tool-results` with a 64 MiB entry
limit and a 128 MiB cache budget. Old handles can expire; do not retry a
mutation merely to retrieve its output.

The built-in agent retains its original transcript but constructs a bounded
view for each provider request. It prioritizes the current user request,
the latest tool exchange, and earlier user requirements. Older evidence and
images are replaced by explicit `session.read` references. That tool can
retrieve a message, a tool result's JSON pointer, or one archived image.
When earlier requirements cannot fit, the view identifies their archive
indices and tells the agent to read them before making assumptions. This is
deterministic context reduction, not an LLM-generated semantic summary.

`CUT_AGENT_INPUT_BUDGET` controls the input allowance (default 65536 units).
The allowance conservatively charges text by UTF-8 bytes plus overhead and
reserves 16384 units per image; it is not an exact vendor token count. Choose
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
target/release/cut-mcp --dir ./my-project --full --print-config
```

`--print-config` prints pretty JSON with the absolute path of the running
`cut-mcp` executable and an absolute project path. Relative project paths are
resolved against the current working directory; the project need not exist.
This mode exits without starting `cutd` or changing the project. The full
profile adds `--full` to the generated arguments, including when selected by
`CUT_MCP_PROFILE=full`. `CUT_PROJECT_DIR` supplies the default project path
when `--dir` is omitted.

To run the stdio server directly:

```bash
target/release/cut-mcp --dir ./my-project
target/release/cut-mcp --dir ./my-project --full
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

The core profile contains the stable editing loop:

| Tool | Purpose |
|---|---|
| `project.outline` | Read the structural sequence, track, clip, caption, effect, and marker hierarchy |
| `timeline.query` | Inspect tracks, clips, source ranges, and placement |
| `timeline.check` | Check source availability, timeline coverage, compiler findings, and capability mismatches |
| `composition.check` | Inspect a local HTML composition and optionally validate a schema v1 manifest plus sparse instance overrides |
| `composition.catalog` | Discover local compositions and their manifest/meta/storyboard metadata |
| `composition.preview` / `composition.render` | Render a composition through the configured external bridge with deterministic caching |
| `timeline.capture` | Capture a deterministic offline timeline range as a frame or filmstrip |
| `timeline.apply` | Stage one canonical v2 command or atomic batch for human review |
| `media.probe` | Run info, silence, scenes, exact frames, filmstrips, waveform, transcription, selected-range listening, or a bounded multi-modal `observe` envelope |
| `media.asset` | Register an existing local artifact and optionally stage import/replacement as one proposal |
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

`composition.check` remains read-only. Pass `manifest` with a project-relative
JSON sidecar to validate `schemaVersion: 1`, template identity, explicit
output metadata, and typed `text`, `color`, `number`, `boolean`, `enum`,
`image`, `video`, or `audio` slots. Pass `instance` with a second JSON sidecar
to validate sparse overrides against that manifest:

~~~json
{
  "manifest": "composition.manifest.json",
  "instance": "launch.instance.json"
}
~~~

The HTML entry can expose slot bindings through `data-var-slot-id` or
`data-var="slotId"`. Media overrides are asset-id strings; the check does not
import media, update the canonical project, or resolve render provenance.

`composition.catalog` performs bounded, sorted, non-executing discovery of
HTML entries. A composition directory may contain `index.html`,
`composition.manifest.json` (or `manifest.json`), `meta.json`,
`STORYBOARD.md`, and a local thumbnail. Catalog results include template
identity, output, slots, tags, validation status, and those local references;
the catalog never changes the canonical project.

`composition.preview` and `composition.render` use an administrator-configured
external renderer rather than embedding a browser in Cut. Configure both a
renderer executable and a pinned version:

```bash
export CUT_COMPOSITION_RENDERER=/absolute/path/to/cut-composition-renderer
export CUT_COMPOSITION_RENDERER_VERSION=1.0.0
```

The bridge starts that executable without a shell, clears its environment, and
sends one bounded JSON request on stdin from a temporary `inputs/` snapshot.
The worker must write the requested `artifact.path` in its working directory
and return one JSON object acknowledging `bridgeVersion`, `rendererVersion`,
`cacheKey`, `inputFingerprint`, `artifact`, and `ok: true`. Preview requests
default to one PNG frame; render requests default to an MP4 over the complete
composition frame range. Missing or changed media asset IDs, invalid or
non-deterministic compositions, missing acknowledgements, invalid output
signatures, timeouts, and partial artifacts fail closed; there is no authored
media fallback.

Completed artifacts live under `.cut/cache/derived` and are keyed by the
normalized template/source/binding/asset fingerprints, renderer identity and
version, output metadata, mode, frame range, and format. Completion is
published atomically and the artifact is registered in `.cut/cache/assets.json`
with template, renderer, and cache provenance. These tools only write
regenerable cache/provenance artifacts; `project.cut`, revisions,
proposals, and the timeline remain unchanged.

For agent refinement, `media.probe` with `action: "observe"` accepts a required
bounded `opts.start` plus `opts.end` or `opts.duration`, and an optional
`opts.evidence` list (`frames`, `filmstrip`, `waveform`, `transcript`, or
`listen`). It returns the selected range, source metadata, requested frame
times, and the existing cached derivative results in one evidence envelope.
The default evidence is a filmstrip. Passing an imported source ID in `src`
resolves it through the current project before probing.

For a complete local rough-cut pass, the agent first uses `media.probe` for
coarse evidence and then calls `workflow.roughcut` with the selected ranges:

```json
{
  "goal": "Keep the strongest opening moments",
  "selections": [
    {"sourceId": "source-1", "start": 12, "end": 18, "rationale": "clear opening statement"},
    {"sourceId": "source-2", "start": 4, "end": 9, "rationale": "supports the opening visually"}
  ]
}
```

Each selected range is targeted, checked against the candidate timeline, and
staged as one typed proposal with a bounded before/after preview. Without
selections the workflow returns `planning_needed`; it never chooses a first
clip or source. The proposal remains pending until the user accepts it. An
optional `duration` is an output timeline upper bound after speed conversion;
the sum of selected timeline durations must fit it, or the workflow returns an
argument error without shortening the agent's selections. The preview starts
at the proposed timeline ranges; when ranges are farther apart than the
120-second capture bound, the response identifies the representative range
and reports which selections were covered.

Embedded agent tools and both external MCP profiles omit direct `media.fetch`,
`media.import`, and timeline undo/redo calls because those paths bypass review.
External calls to these tools return `AGENT_CANONICAL_BYPASS`. External
`timeline.apply` calls create a pending proposal through the same review boundary
as embedded agent edits; the canonical project and revision remain unchanged
until a human accepts the proposal in the UI. Use `media.asset` with an
existing local file to stage an import/replacement, and leave history changes
to the human UI. Observation references retain the selection index, rationale,
and optional coarse observation; project-bound references carry the current
revision and are rechecked against source path and file identity at acceptance.

already-produced local file can be registered with `media.asset`; it is cached
with request/content fingerprints under `.cut/cache/assets.json`, whose
read-modify-write is locked and atomically replaced. Asset records are
immutable: reusing a request fingerprint for a different content hash returns
`ASSET_REQUEST_CONFLICT`, and staging verifies the file hash again. With
`import: true`, the asset import and optional `replaceClipId` are staged
together and leave the canonical project unchanged until proposal acceptance.
Cut does not invoke cloud media generation from this path.

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
