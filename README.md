# Industrial Kerf

Industrial Kerf is a native, agentic video editor. `kerf` opens the Qt6 editor by default,
on its project launcher when no project is named;
pass `--terminal` for the project-scoped terminal agent. `kerf-mcp` is the
project-scoped MCP server and primary external-agent integration surface.
External Claude, Cursor, and Codex-like clients connect through `kerf-mcp`
using their own agent configuration; they do not need a provider API key
configured in Kerf.

## Architecture

`kerfd` is the only owner of project state, revisions, history, idempotency
receipts, proposals, and render jobs. The native editor, terminal agent, and
MCP server are adapters over the same project-scoped IPC boundary.

The canonical project is the `project.kerf` text document
([the Kerf language](docs/kerf-language.md)). Edits are edit scripts or typed
operations; agent proposals carry their script and the diff of the document
they would produce. Because the project is text, people share it through Git
and GitHub: a sync commits, merges, and pushes, and `project.kerf` merges by
the ids of what it holds ([shared projects](docs/collaboration.md)).

```text
Qt6 editor ──────────────────────────┐
Rust agent and CLI ──────────────────┼─> kerfd ─> kerf-model ─> project.kerf
Rust MCP server ─────────────────────┘     │
                                           └─> .kerf document ─> render compiler,
                                                                playback, previews
```

Clients start `kerfd` on demand, one per project. The editor, the MCP server,
the terminal agent, and a running render hold it open; once they are all
gone and it has no media left to index, it exits after five seconds. A
one-shot command such as `kerf sync` holds nothing, so `kerfd` waits ten
minutes after the last request instead; `KERFD_IDLE_TIMEOUT`, in the
environment `kerfd` starts with, sets that wait in seconds, and `0` turns it
off. The next request starts it again.

Renders, playback, and previews read the model. A render job and a proposal
preview carry the `.kerf` document with media paths resolved against the
project, and the render compiler lowers its first sequence to one FFmpeg
graph with exact times: picture tracks stack in order, sound comes from
audio tracks, transitions play across their cuts using the clips' handles,
nested sequences composite on their own canvas, and effects, transitions,
and generators lower from their catalog definitions. A feature without a
lowering is reported with the clip it is on. Native playback plays the same
sequence as cuts, running a media clip's raster effects live on each
picture (except keys and mattes, whose transparency needs the tracks
below, and time effects, which read the clip's other frames), and asks for
a rendered preview whenever it would show something different. Editor snapshots carry the document and a timeline
view of it; the editor sends its edits back as typed operations.

Kerf draws with its own renderer (`kerf-draw`, drawn by `kerf-compose`): it
draws Kerf's model directly in linear light, with drawing carried from
[FilmCraft](https://github.com/storytold/filmcraft) (MIT/Apache), for
renders, previews, and live playback. A video editor's
transitions (pushes, whips, zooms, flashes, light leaks, glitches, irises)
and effects (Lumetri, blurs, keys and track mattes, glows, distortions, moves,
burn-ins, 360° video) are FilmCraft's, carried into `kerf-fx`. Kerf also keeps
24 effects from [EffectCraft](https://github.com/storytold/effectcraft)
(MIT/Apache, in `kerf-raster-fx`) that FilmCraft has nothing like: restoration, key and matte clean-up, curves and finer color, a few
blurs and distortions, and the audio visualizers. An effect's parameters are
written in the catalog's terms (a point as an `_x` and a `_y` across the
picture, a file as a path, another track by id, a mask by name), and an id
EffectCraft had that Kerf no longer does is an error that names what to use
instead. Playful effects are looks ([docs/looks.md](docs/looks.md)): text
files in the Kerf language that combine those effects, keyframes, and masks
behind a few parameters, bundled with Kerf or kept in a project's
`.kerf/looks`, and expanded when a program draws.
The graph can still render a program without FilmCraft's effects and
transitions: it first bakes a clip's kept EffectCraft effects into a lossless
file with transparency in `.kerf/cache/derived`, and plays that in the
clip's place. Media can carry transparency too (`alpha=true`, found from the
pixel format on import), which compositing keeps. `kerf-fx` generates the
catalog tables; after changing the effects, run
`UPDATE_CATALOG=1 cargo test -p kerf-fx`.

Editor actions are commands in one registry (`kerf-commands`): each has an id,
a label, its menu, shortcuts in the Kerf, Premiere Pro, and Final Cut Pro
keymaps, and an enabled state that says why it is unavailable. The editor
builds its menus, shortcuts, and toolbar from it, and engine commands
resolve to the typed operations or runtime requests that carry them out.

FFmpeg, ffprobe, and yt-dlp remain controlled native sidecars. Editing
playbooks are data in `skills/*/SKILL.md` and are loaded by `kerf-skills`.

## Build and run

```bash
cargo build --release -p kerf-runtime --bin kerfd
cargo build -p kerf-qt-bridge
cmake -S apps/kerf-qt -B target/qt
cmake --build target/qt
cargo build --release -p kerf-cli --bin kerf
cargo build --release -p kerf-mcp --bin kerf-mcp
```

Or build and install everything: `kerf`, `kerfd` and `kerf-mcp` to
`~/.local/bin` (`KERF_LOCAL_BIN` moves them), and the editor. On macOS the
editor is an app, `~/Applications/Industrial Kerf.app` (`KERF_LOCAL_APPS`
moves it), carrying its own libraries and `kerfd`; it replaces an older
install's app, and the plain `~/.local/bin/kerf-qt` an older install left.
Elsewhere it is `~/.local/bin/kerf-qt`, with its libraries in
`~/.local/lib/kerf` (`KERF_LOCAL_LIB`). The install does not refer back to
the checkout, so cleaning `target` is safe; Qt and FFmpeg still come from the
system:

```bash
scripts/install-kerf-local.sh        # or: cli, qt
```

Open the Qt editor. Without a project it shows the project launcher, where
you make a project from a template or a preset, clone one from GitHub, or
open a recent one; `--dir` (or `KERF_PROJECT_DIR`) opens a project directly:

```bash
target/release/kerf
target/release/kerf --dir ./my-project
```

`kerf` finds the editor beside itself, then on macOS in the installed app,
then in `target/qt` (on macOS in its bundle, `target/qt/kerf-qt.app`); run
from the checkout, it tries the checkout's build before the app.
`KERF_QT_BIN` overrides it. It finds `kerfd` beside itself or in `target`.
`scripts/run-kerf-qt.sh [project]` builds the debug editor and opens it the
same way.

The editor is dark only (one graphite ramp and one blue accent) and fills the
screen edge to edge. It has four workspaces (Review, Edit, Finish, Deliver),
shows a 16:9 picture in an exact MacBook Pro 14" frame and 9:16, 1:1 and 4:5 in
an exact iPhone 16 Pro frame, and keeps the review as comment threads (from
people, Kerf, agents over MCP and QC) in the project's review log. On macOS it
opens `kerf://<project>/rev<N>@HH:MM:SS:FF` review links. See
[the editor's README](apps/kerf-qt/README.md).

Make a project and share it from a terminal (see
[shared projects](docs/collaboration.md)):

```bash
target/release/kerf new "My Film" --location ~/Movies --vertical --github
target/release/kerf sync --dir ~/Movies/"My Film"
```

Run the terminal workflow:

```bash
target/release/kerf --terminal --dir ./my-project video.mp4
target/release/kerf --terminal --dir ./my-project -p "trim the first clip to two seconds"
target/release/kerf --terminal --dir ./my-project --skills
target/release/kerf --terminal --dir ./my-project --generate-docs
target/release/kerf --terminal --dir ./my-project --mcp
target/release/kerf --terminal --dir ./my-project --mcp-tools -p "use the connected research tool"
```

In editor mode, `kerf` imports a local media path or an HTTP(S) URL before
opening the native window; in a project with a `media/` folder, or a shared
one, the file is copied (or downloaded) into `media/` first, as the editor's
import does. The terminal workflow, `kerf preset`, `kerf sync`,
and a media import use the current directory unless `--dir` or
`KERF_PROJECT_DIR` names a project. The editor's Kerf Assistant (the chat beside the picture) uses the same provider
configuration as the terminal agent. `--kerfd PATH` selects an explicit
daemon; the `KERFD_BIN` environment variable does the same.

`--mcp` lists the native catalog and any servers configured in
`~/.kerf/mcp.json` or `<project>/.kerf/mcp.json`. `--mcp-tools` (or
`KERF_ENABLE_MCP=1`) attaches those inbound stdio servers through `mcp_search`,
`mcp_describe`, and `mcp_call`; configured tool allowlists are enforced.
The Qt editor's Settings tab can enable the same attachment for its Chat,
edit the project config, or import a Cursor config. `KERF_ENABLE_MCP=1` remains
available as an environment fallback. Qt remains a presentation layer.

## Agents

Native editing presets are exported from **File → Export Editing Preset…**
as one `.kerfpreset` file containing media slots, `.kerf` editing code, and
the project's own looks it uses.
**File → Import Editing Preset…** binds new media and stages the same pattern
as a proposal. A project's `presets/` folder is its preset library, shared with
the project's repository. See [native preset format and reuse](docs/presets.md)
and [shared presets and media slots](docs/collaboration.md#shared-presets-and-media-slots).

Agents edit the same `.kerf` document people do (see
[the Kerf language](docs/kerf-language.md)). Their context is the language
reference, generated from the feature catalog, and the current document,
annotated with each clip's end and source out. The built-in agent reads the
document again before every model request, so it always sees the current
revision and the state of its proposal. There is no tool discovery; four
tools cover the work:

| Tool | Purpose |
|---|---|
| `look` | No arguments: the annotated document and proposal state. `media`: that media's index — where it is used, scene cuts, pauses, loudest moments, sound onsets, and tempo, and with `transcript: true` what is said (`words: true` times each word). `sheet: N`: a contact sheet of N frames over `from`–`to`, of `media` or the sequence, as one image. `at`: up to eight full-size frames — source frames of `media`, or program frames of the sequence. `transcript: true` without `media`: what the sequence says as it plays. `sequence` picks a sequence other than the first. `proposal`: the same, as a pending proposal would make the project. `effects`: the catalog entries for a look or moment ("neon glow", "zoom hit"), best first, each with when to use it; `effect`: one entry's parameters |
| `edit` | Run an edit script. It becomes a proposal, returned with the document diff, check findings, and a before/after preview: the frames either side of each cut it made or moved, and the middle of a clip it changed in place. `extend` runs the script on top of a pending proposal and makes the two one proposal; `revise` replaces it |
| `render` | Render the sequence (or another `sequence`), or `from`–`to`, as a draft or full-quality file — of the project, or of a pending `proposal`; `job` reads progress, `cancel` stops it |
| `note` | Post a comment to the project's review log (`review/comments.jsonl`, beside the document): `body`, optionally `at`/`duration` on the program, `parent` to reply in a thread, `proposal` to link it to one, `tags`, and `blocking`. It shows in the editor's comments panel authored by the client (Claude Code, Cursor, Codex, or Kerf itself), never makes a pending proposal stale, and never enters undo |

An edit's check findings include what an editor would catch, as warnings:
black stretches with nothing on screen, picture that leaves the frame's
edges black, flash frames shorter than three frames, and sound cut inside a
word (known from transcripts of that media already made; nothing is
transcribed for the check).

Times are timecode (`01:00:04:12`), frames (`96f`), or seconds (`4.5s`):
media positions with `media`, sequence positions otherwise. The project
changes only when a person accepts a proposal. In the editor, Kerf is asked
about the selection (⌘J): the request carries the clip or range it is about,
and the proposal shows on the timeline to accept or reject in place.

Edit scripts can place by anchor instead of timecode (`at=end`,
`at=c3.end`, `to=c3.start-6f`, a marker's id such as `at=beat1+2f`, or
`at=c3@00:00:07:29` for where `c3` plays that time of its media), trim to
a source position
(`trim c1 out src=00:00:07:29`, such as a word's time), and caption what a
clip says with `captions c1 on=S1 [words=<n>]`, which uses a transcript of
its media made earlier and stages the caption lines it becomes.

When media is imported, `kerfd` indexes it in the background — one FFmpeg
pass for scene cuts, pauses, and a loudness envelope from which loudest
moments, onsets, and tempo are found — and makes a proxy at most 960 pixels
wide for pictures wider than that. Draft renders and frames up to that width
use the proxy when it is ready; full renders and program frames use the
original. Indexes and proxies are cached in `.kerf/cache/derived`.

Transcripts come from local `whisper.cpp` (`KERF_WHISPER_BIN` and
`KERF_WHISPER_MODEL`, described below); media is never uploaded. Scene cuts
and pauses are cached with the media's other derivatives in
`.kerf/cache/derived`.

Skills hold editorial guidance only — what makes a good short, how a meme
lands — not tool procedures. Bundled skills are compiled into `kerf-skills`;
user (`~/.kerf/skills`) and project (`.kerf/skills`) skills override them by
name. Every skill's guidance is part of the agent's instructions.

The built-in agent keeps its whole transcript and sends it to the provider
append-only, so the system prompt and everything already sent stay the same
from request to request: the prompt cache holds, and the signatures on a
model's thinking blocks stay valid. The project document and proposal state
are added to the conversation as a new message whenever they change, not
rewritten into the system prompt. Anthropic requests mark the system prompt
and the latest message for prompt caching and send thinking blocks back
unchanged.

When the transcript outgrows the input allowance, it is compacted once:
tool results before the latest exchange become `session.read` references
(frames and previews included), earlier project snapshots become notes, the
current project is added again, and earlier thinking is left out. User
requests stay whole. From there the transcript grows append-only again.
`session.read` retrieves a message, a tool result's JSON pointer, or one
archived image. A single tool result whose text is too long is archived
when it arrives, keeping its frames and previews. This is deterministic
context reduction, not an LLM-generated summary.

`KERF_AGENT_INPUT_BUDGET` controls the input allowance (default 196608 units).
The allowance conservatively charges text by UTF-8 bytes plus overhead and
reserves 4096 units per image; it is not an exact vendor token count. Choose
an allowance below the selected model's context window, leaving room for
output and reasoning. A context-limit response causes one retry with half
the allowance, and a provider that refuses earlier thinking blocks causes
one retry without them; neither re-executes tools. A user request or fixed
catalog that cannot fit produces an explicit error instead of silent
truncation.

Inbound MCP transport limits each message to 4 MiB. On Unix, it times out
after 30 seconds (`KERF_MCP_TIMEOUT_MS` overrides it) and running calls observe
the agent's cancellation flag. A cancelled external operation
may have already taken effect; verify its state before retrying it.

The Qt bridge, daemon, decoder, agent proposal, and restart smoke test are
available at:

```bash
cargo build -p kerf-qt-bridge -p kerf-runtime --lib --bin kerfd
python3 scripts/test-kerf-qt-bridge.py
```

## Kerf Cloud and optional embedded-agent providers

The editor is free. Kerf Cloud, the hosted model, is the paid tier: Settings →
AI signs in to it, and the service meters requests and enforces plans. It is
not deployed yet; [docs/cloud.md](docs/cloud.md) is the contract the editor
follows, and `KERF_CLOUD_URL` points the editor at a service. Until then, use
your own key under Settings → Advanced, as described below.

Provider APIs are optional and only power Kerf's embedded assistant (and its
terminal agent workflow). They are not required for external agents using
`kerf-mcp` or for native editing.

| Provider | Key | Transports |
|---|---|---|
| Anthropic | `ANTHROPIC_API_KEY` | Messages SSE |
| OpenAI | `OPENAI_API_KEY` | Chat Completions or Responses |

Provider selection uses `--provider`, then the configured key. Models and
OpenAI-compatible endpoints can be set with CLI flags or `~/.kerf/config.json`.
Keep credentials outside project directories and never commit them.

```bash
export OPENAI_API_KEY=...
export OPENAI_BASE_URL=https://openrouter.ai/api/v1
target/release/kerf --provider openai --model anthropic/claude-sonnet-4 \
  --dir ./my-project -p "make a short vertical preview"
```

Responses mode and reasoning effort are selected with `--api-mode responses`
and `--reasoning-effort low|medium|high|...`. With Anthropic, the default
model is `claude-opus-5-5`, and `--reasoning-effort` sets its effort
(`none` and `minimal` ask for `low`).

## MCP

`kerf-mcp` speaks JSON-RPC MCP over stdin/stdout. Diagnostics go to stderr and
all project mutations go through `kerfd`.

Generate a generic MCP client configuration, then copy the JSON into your
external client's MCP settings:

```bash
target/release/kerf-mcp --dir ./my-project --print-config
```

`--print-config` prints pretty JSON with the absolute path of the running
`kerf-mcp` executable and an absolute project path. Relative project paths are
resolved against the current working directory; the project need not exist.
This mode exits without starting `kerfd` or changing the project.
`KERF_PROJECT_DIR` supplies the default project path when `--dir` is omitted.

To run the stdio server directly:

```bash
target/release/kerf-mcp --dir ./my-project
```

Keep `kerfd` beside `kerf-mcp`, put it on `PATH`, or set `KERFD_BIN` in the client
process environment. The generated configuration has this shape:

```json
{
  "mcpServers": {
    "kerf": {
      "command": "/absolute/path/to/target/release/kerf-mcp",
      "args": ["--dir", "/absolute/path/to/my-project"]
    }
  }
}
```

`kerf-mcp` serves the agent tools above — `look`, `edit`, and `render`. The
server instructions carry the language reference and editorial guidance;
`look` with no arguments, or the resource `kerf://document`, returns the
annotated document, and `kerf://reference` the reference. Frames and previews
arrive as MCP image content, as JPEG. An `edit` from an external client is a proposal
like any other: the project and revision stay unchanged until a person
accepts it in the editor. `KERF_MCP_NAMESPACE` prefixes the tool names (for
example `kerf.look`).

Local transcription is optional and never uploads media. Install a
`whisper.cpp` `whisper-cli` binary and configure a local GGML model before
using `look` with `transcript: true`:

```bash
export KERF_WHISPER_BIN=/absolute/path/to/whisper-cli
export KERF_WHISPER_MODEL=/absolute/path/to/ggml-base.bin
target/debug/kerf-engine probe listen input.mp4 --start 12 --duration 30 --project-dir ./my-project
```

Set `KERF_WHISPER_CPP_VERSION` and `KERF_WHISPER_MODEL_VERSION` when packaging
or pinning sidecar/model versions.

`kerf --generate-docs` writes deterministic, machine-owned protocol, tool,
skill, and current-project summary files under
`.kerf/docs/generated/`. Keep editorial guidance in
`.kerf/docs/conventions.md`; the generator never overwrites it or a root
`AGENTS.md`. If a root pointer is useful, pass the explicit
`--add-agents-pointer` opt-in; it appends an idempotent pointer without
replacing existing instructions. The project summary lists the sequences
and media of the current document and cannot change project state.

## Interchange

`kerf-interchange` converts the program sequence of a project to a
frame-exact timeline (positions in frames of the sequence, media times in
frames of the media) and invokes the configured native or external adapter
through a small JSON bridge, reporting what the conversion leaves out.
OpenTimelineIO conversion uses the bundled
`crates/kerf-interchange/python/otio_bridge.py` and requires an environment
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

## Licenses

Kerf is MIT licensed ([LICENSE](LICENSE)). The code it carries from
EffectCraft and FilmCraft is MIT OR Apache-2.0, and its text engine bundles
fonts under the SIL OFL 1.1; [licenses/README.md](licenses/README.md) lists
where each part lives and holds the license texts and notices.
