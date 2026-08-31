# `@cut/gui`

React viewer for Cut. It loads `project.cut.json` from the host, plays video, and
lets you compare before/after results.

The bottom timeline is a read-only map for seeking and clip selection. Color is
the only inspector that applies direct mutations from the UI. All other edits go
through the agent (`POST /api/agent`) or revision-checked commands
(`POST /api/command`).

The browser app talks to the host through the following endpoints:

- `GET /api/state` loads the project, revision, sources, and history flags.
- `POST /api/command` applies revision-checked timeline commands.
- `POST /api/undo` and `POST /api/redo` navigate project history.
- `POST /api/agent` runs an agent request and returns its events/summary.
- `POST /api/render` plus `GET /api/render/:jobId` manages preview and full renders.
- `GET /api/media/:sourceId` streams a source; `GET /api/output/:jobId` streams a render.

If the host is unavailable, the UI remains usable as an explicit connection/error
state rather than inventing a project or pretending an edit was saved. Set
`VITE_CUT_API_URL` when the API is served from a different origin.

## Run

Dependencies are declared locally in this package. From the repository root, install dependencies when the workspace is ready, then run:

```bash
npm --workspace @cut/gui run dev
```

To produce a production build:

```bash
npm --workspace @cut/gui run build
```

The Vite server uses port `4174` by default. The Color inspector preserves all
existing clip effects and sends a canonical `update_clip.patch.effects` command
for the selected video clip after a short debounce.
