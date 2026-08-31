# `@cut/desktop`

The desktop build is a Tauri shell around the React editor and a Rust-owned,
long-lived editing runtime. Tauri owns the window and runtime lifecycle; Rust
owns the canonical v2 project snapshot, semantic command transactions,
revision checks, undo/redo, persistence, and render jobs.

```text
React/TypeScript ── typed Tauri commands ── Rust runtime ── cut-engine/FFmpeg
Browser/MCP ───────────────────────────── external HTTP adapter (separate)
```

The desktop command bus accepts canonical commands only. Every mutation carries
`actor`, `baseRevision`, and `idempotencyKey`; a successful transaction advances
the revision once. Reusing a key replays the original result, while reusing it
for a different request returns `IDEMPOTENCY_CONFLICT`. The same bus is the
integration point for UI and agent actors. Legacy aliases and host-specific
HTTP behavior are not implemented in the Rust runtime.

Media import is also a command-bus transaction (`desktop_import_media`) and
therefore carries the same actor/revision/idempotency envelope. The public
apply surface has no `replace_project`; full-project replacement is reserved
for bootstrap/migration code outside this runtime command protocol.

The `cut-runtime` binary exposes the same runtime as a persistent JSONL process
for Node/MCP adapters. Requests use `protocolVersion: 1` and operations
`get_state`, `apply`, `import`, `undo`, and `redo`; responses are versioned and
contain either a state or a structured error.

Idempotency records currently live for the desktop process lifetime; callers
must keep request keys stable while retrying an operation. Persisted project
history remains the source of truth across launches, while durable operation
receipts are a follow-up hardening step.

## Development

Install the workspace dependencies and run:

```bash
npm --workspace @cut/desktop run dev
```

This starts Vite on port `4174` and opens the Tauri window. The development
project is stored in `.cut/desktop-project` at the repository root.

`CUT_PROJECT_DIR` can be set to choose the project directory. Without it,
development uses `.cut/desktop-project` and packaged builds use the platform's
Tauri application-data directory. Rendering and probing use `CUT_ENGINE_BIN`,
or the checkout's debug/release `cut-engine` binary when available.

## Production build

```bash
npm --workspace @cut/desktop run build
```

Use `tauri build --no-bundle` while developing the shell itself. Standalone
installer packaging is intentionally deferred until the host sidecar build is
selected for each target platform.

`@cut/gui` uses the Tauri transport when running inside Tauri and uses the
browser HTTP adapter only in browser/MCP hosting. The desktop runtime does not
start the TypeScript HTTP host. External agent orchestration must be supplied
as an explicit adapter and submit canonical semantic commands with
`actor: "agent"` to this runtime rather than maintaining a second project
state. Set `CUT_AGENT_BIN` to an executable implementing the versioned JSON
stdin/stdout agent contract (`{ prompt, state, actor }` in; `{ protocolVersion: 1,
transactions: [{ idempotencyKey, commands }] }` out)
to enable the desktop agent command. Local media and render output use Tauri's
scoped `asset:` protocol and the native dialog picker; browser `/api/media/*`
and `/api/output/*` remain external-adapter concerns. YouTube URL import is
intentionally unavailable in desktop mode until a downloader sidecar is added.
