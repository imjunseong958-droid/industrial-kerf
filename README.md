# cut

pi-style agentic video editor. Chat in your terminal; the agent edits video via ffmpeg.

```
npm install   # in repo root
ANTHROPIC_API_KEY=sk-... npx cut video.mp4
```

## Usage

```
cut [video-or-url] [options]

Options:
  -k, --key <key>        LLM API key (or env ANTHROPIC_API_KEY / OPENAI_API_KEY)
  --provider <name>      anthropic | openai  (default: auto from available key)
  --model <model>        model id (default: claude-sonnet-4-20250514 / gpt-4o)
  --dir <path>           session directory (default: .)
  --cookies <path>       yt-dlp cookie file (default: ~/yolomodellayout/youtube.com_cookies.txt if present, or CUT_COOKIES env)
  -p "<instruction>"     print mode: single-shot, no REPL
  --skills               list loaded skills
  --mcp                  list configured MCP servers and their tools
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
npx cut --provider openai --model anthropic/claude-sonnet-4 video.mp4

# Local Ollama
export OPENAI_BASE_URL=http://localhost:11434/v1
export OPENAI_API_KEY=ollama
npx cut --provider openai --model llama3.1 video.mp4
```

Or persist in `~/.cut/config.json`:

```json
{ "provider": "openai", "apiKey": "...", "model": "gpt-4o" }
```

## Tools

- **fetch** — YouTube/URL download via yt-dlp (≤1080p, best audio, ko/en subtitles), cookie support
- **probe** — info / silence detection / scene detection / frame extraction
- **edit** — timeline ops: trim, split, remove_ranges, set_speed, set_volume, add_subtitles, remove_overlay
- **render** — draft preview (360p/15fps fast) or full render with progress
- **bash** — escape hatch, sandboxed to the session dir

The timeline (`project.cut.json`) is the single source of truth — human-editable and git-friendly.

## Skills & MCP

- Skills: SKILL.md files in `skills/` (bundled: shorts, meme), `~/.cut/skills/`, `<session>/.cut/skills/`
- MCP servers: configure in `~/.cut/mcp.json` → `{ "mcpServers": { "name": { "command": "npx", "args": ["-y", "@some/mcp-server"] } } }`

## Development

```bash
npm run typecheck && npm test    # tsc -b + vitest across all packages
```
