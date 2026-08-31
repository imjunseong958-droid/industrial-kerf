# cut

Agentic video editor. Run `lcut` to open the viewer-first local GUI; use
`--terminal` when you want the original REPL workflow.

## Architecture

IR is `project.cut.json` (v2). Rust `cut-core` and `cut-engine` are the
deterministic/native engine; TypeScript packages own product orchestration and
the UI-facing project model. `cut-mcp` is the stdio MCP host. Playbooks live in
`skills/*/SKILL.md`, and `@cut/gui` is shared by browser and Tauri shells.

It is not a full NLE and not a generic coding-agent platform. A coding agent
(Cursor, Claude Code, Codex) is a second client of `cut-mcp`, not the product
runtime.

```
npm install && npm run link:global   # in repo root
lcut video.mp4
```

To make the workspace binary available as `lcut` in any terminal, link it once:

```bash
npm run link:global
lcut video.mp4
```

The GUI starts without asking for an API key. Configure a provider through the
environment or `~/.cut/config.json`; the agent panel reports a missing key only
when you submit a request. The server stays alive until `Ctrl-C` and opens the
local URL in your browser automatically. Use `--no-open` to print the URL
without opening a browser. The GUI is viewer-first: playback and before/after
verification, a read-only timeline map for seeking, and a Color inspector that
is the only direct UI mutation path.

## Usage

```
lcut [video-or-url] [options]

Options:
  --terminal               use the terminal REPL instead of the GUI
  --no-open                do not open the GUI URL automatically
  --host <host>            GUI bind host (default: app package default)
  --port <port>            GUI port (0 chooses an available port)
  -k, --key <key>        LLM API key (or env ANTHROPIC_API_KEY / OPENAI_API_KEY)
  --provider <name>      anthropic | openai  (default: auto from available key)
  --model <model>        model id (default: claude-sonnet-4-20250514 / gpt-4o; Responses: gpt-5.6-luna)
  --dir <path>           session directory (default: .)
  --cookies <path>       yt-dlp cookie file (default: ~/yolomodellayout/youtube.com_cookies.txt if present, or CUT_COOKIES env)
  -p "<instruction>"     print mode: single-shot, no REPL
  --skills               list loaded skills
  --mcp                  list configured MCP servers and their tools
  --mcp-tools            attach inbound MCP to the terminal agent (or CUT_ENABLE_MCP=1)
  -h, --help             show help and exit
  -v, --version          show version and exit
```

Examples:

```bash
# Open the GUI and load a local video
npx lcut video.mp4

# Start the GUI on a known port without opening a browser
npx lcut --no-open --host 127.0.0.1 --port 4312

# Keep the terminal-only agent REPL
npx lcut --terminal video.mp4
npx lcut --terminal video.mp4 -p "remove the silence"
```

## Providers

| Provider | Key env | Notes |
|---|---|---|
| anthropic | `ANTHROPIC_API_KEY` | default |
| openai | `OPENAI_API_KEY` | any OpenAI-compatible endpoint |

OpenAI-compatible endpoints (OpenRouter, Ollama, LM Studio…):

```bash
# OpenRouter
export OPENAI_API_KEY=sk-or-...
export OPENAI_BASE_URL=https://openrouter.ai/api/v1
npx lcut --provider openai --model anthropic/claude-sonnet-4 video.mp4

# Local Ollama
export OPENAI_BASE_URL=http://localhost:11434/v1
export OPENAI_API_KEY=ollama
npx lcut --provider openai --model llama3.1 video.mp4
```

Or persist in `~/.cut/config.json`:

```json
{ "provider": "openai", "apiKey": "...", "model": "gpt-4o" }
```

OpenCode Zen or Go Responses API can be selected in the same local config. The
`baseUrl` is the API root (the SDK appends `/responses`), and `reasoningEffort`
maps to the Responses API's `reasoning.effort` field:

```json
{
  "provider": "openai",
  "apiKey": "...",
  "baseUrl": "https://opencode.ai/zen/go/v1",
  "apiMode": "responses",
  "model": "gpt-5.6-luna",
  "reasoningEffort": "low"
}
```

Keep this home config outside the repository; the project-local
`.cut/config.json` path is ignored as an additional safeguard if you use one.
Do not commit API keys.

## Tools

`project.cut.json` is the project file. It is human-editable and git-friendly.
Edits use canonical v2 commands (or atomic command batches) with
`expectedRevision` conflict checks, in-memory undo/redo, and atomic persistence.
At the MCP boundary, legacy v1 `op`/`ops` are translated; v2 is persisted after
the first mutation.

Each row lists the agent-facing name (default GUI agent; `lcut --terminal`
overlaps except where noted) and the MCP dotted name served by `cut-mcp`.

| Agent tool | MCP tool | Notes |
|---|---|---|
| `fetch` | `media.fetch` | YouTube/URL download via yt-dlp (≤1080p, best audio, ko/en subtitles), cookie support; appends to the project |
| | `media.import` | Import a local file and append it (`cut-mcp` only; not exposed on the built-in GUI agent) |
| `probe` | `media.probe` | `action`: `info`, `silence`, `scenes`, or `frames` (PNG extraction, up to 8 frames with `{time,path}` metadata) |
| `timeline` | `timeline.query` | Inspect sources, clips, placements, and durations before editing |
| `project_edit` | `timeline.apply` | One v2 command or atomic batch; legacy `op`/`ops` accepted at the MCP host |
| `timeline_undo` / `timeline_redo` | `timeline.undo` / `timeline.redo` | Undo/redo project commands (shared GUI and `--terminal` catalog) |
| `render` | `render.preview` / `render.start` + `render.result` | `render` waits for completion; `draft=true` uses preview. MCP also exposes `render.status` and `render.cancel` for polling |
| `skill_read` | `skills.read` | Load a playbook body by skill name. Compact index only in the system prompt |
| `track_objects` *(optional)* | `vision.track_objects` | YOLO26 cam/chat/content tracking when vision config is present; weights and labels come only from trusted project/user vision configuration |
| `bash` *(CLI `--terminal` opt-in only)* | | Escape hatch with cwd set to the session dir; enable with `CUT_ENABLE_BASH=1`. **Never** on `cut-mcp` or the GUI agent |

Do not grow the product-agent or cut-mcp tool lists without cause. New timeline
mutations belong on `timeline.apply` / `project_edit` as v2 commands, not as new
MCP tools. New media analysis belongs as a `media.probe` action, not a new tool.
Genre, recipe, and guardrails belong in the `SKILL.md` compact index plus
`skill_read` / `skills.read`. External systems (OTIO and similar) get one
optional MCP group, advertised only when a handler exists.

There is no v1 `edit` tool. Mutations go through `project_edit` / `timeline.apply`;
the host migrates a legacy file on first apply. For screenshot-based zooms,
inspect visual frames with `probe`/`media.probe`, split at source times, then apply
crop updates in the same batch; crop coordinates are normalized 0..1 frame fractions.

Optional OTIO interchange lives in `@cut/interchange` and requires an external
Python environment with `opentimelineio` for every conversion. FCPXML/AAF and
other formats also need their matching OTIO adapter plugins in that environment.

## MCP (`cut-mcp`)

`cut-mcp` is the project-scoped stdio MCP host. Use it when a coding agent
(Cursor, Claude Code, Codex) should edit a Cut project as a second client
alongside (or without) the `lcut` GUI.

```bash
npx cut-mcp --dir ./my-project          # core profile (default)
npx cut-mcp --dir ./my-project --full   # inspect/validate/diff/status/cancel/vision
```

Or set `CUT_MCP_PROFILE=full`. The default core profile is the editing loop:
`timeline.query` / `apply` / `undo` / `redo`, `media.probe` / `fetch` / `import`,
`render.preview` / `start` / `result`, and `skills.read`.

Configure the host in your agent's MCP settings (Cursor / Claude Code style):

```json
{
  "mcpServers": {
    "cut": {
      "command": "npx",
      "args": ["cut-mcp", "--dir", "./my-project"]
    }
  }
}
```

`cut-mcp` writes only MCP protocol frames to stdout; diagnostics go to stderr.
No shell/bash capability is exposed. Render output paths are generated below the
project directory.

Tools served by `cut-mcp` use dotted names. Only capabilities with injected
handlers are advertised. Core is the default stdio profile; `--full` adds
inspect/validate/diff/status/cancel/vision:

| Tool | Purpose | Profile |
|---|---|---|
| `timeline.query` | Read timeline state | core |
| `timeline.apply` | Apply v2 commands/batches or legacy `op`/`ops` | core |
| `timeline.undo` / `timeline.redo` | Project history | core |
| `media.fetch` | Download and append a URL source | core |
| `media.import` | Import a local file and append | core |
| `media.probe` | `info`, `silence`, `scenes`, `frames` | core |
| `render.preview` | Fast preview render job | core |
| `render.start` | Full render job | core |
| `render.result` | Wait for a completed render | core |
| `skills.read` | Load a playbook body by name | core |
| `project.inspect` | Project summary or full JSON | full |
| `project.validate` | Validate `project.cut.json` | full |
| `timeline.diff` | Diff two project snapshots | full |
| `render.status` / `render.cancel` | Render job polling / cancel | full |
| `vision.track_objects` | Object tracking *(only when vision is configured)* | full |

Skills (editing playbooks) load as a compact name/description index. The full
body is read on demand with `skill_read` (product agent) or `skills.read`
(`cut-mcp`). No bash or filesystem path is required. Search order (later wins):

- `skills/` in the repo (bundled: `shorts`, `meme`, `stream-gameplay`)
- `~/.cut/skills/`
- `<project>/.cut/skills/`

To attach *other* MCP servers to `lcut --terminal`, configure `~/.cut/mcp.json`:

```json
{ "mcpServers": { "name": { "command": "npx", "args": ["-y", "@some/mcp-server"] } } }
```

`lcut --mcp` lists discovered tools and exits. Attaching them to the terminal
agent requires `--mcp-tools` or `CUT_ENABLE_MCP=1`; the model then sees only
`mcp_search` / `mcp_call`, not every discovered schema. An optional `tools`
array on a server entry allowlists which underlying names are searchable and
callable.

## Development

```bash
npm run engine:build            # fast debug media engine (without ONNX)
npm run engine:build:release    # optimized media engine (without ONNX)
npm run engine:build:vision     # optimized engine with ONNX tracking
npm run engine:test             # core/media engine tests
npm run gui:build               # build the React viewer
npm run desktop:dev             # run the React viewer in the Tauri shell
npm run desktop:build           # build the unbundled Tauri desktop binary
npm run app:build               # build the local app server
npm run typecheck && npm test   # typecheck + vitest across all packages
```

The lightweight Rust `cut-core` crate owns deterministic timeline compilation;
`cut-engine` owns native media I/O and exposes ONNX tracking only through the
explicit `onnx` feature. TypeScript never builds Rust implicitly. It uses the
debug binary by default, `CUT_ENGINE_PROFILE=release` for an explicit release
selection, or `CUT_ENGINE_BIN` for an exact executable. Release builds use thin
LTO + mimalloc. Decode uses VideoToolbox
on macOS and D3D11VA on Windows when listed. Production encode picks hardware
when available and a fast libx264 preset otherwise. YOLO tracking samples at a
laptop fps, downscales in ffmpeg, and loads `.onnx` in-process (or a sibling
`.onnx` next to a `.pt` checkpoint). There is no TypeScript or Python inference
fallback.
