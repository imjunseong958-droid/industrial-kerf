# Rust migration handoff

This reference records the current repository state and the next implementation units for the Rust-native Cut migration. Start a new Codex task by asking it to read this file and continue from the first unfinished unit.

## Target architecture

The final application removes TypeScript, React, and Tauri from production. The target uses these boundaries:

```text
native editor (winit + egui + wgpu) ─┐
Rust MCP server ──────────────────────┼─> cutd ─> typed domain ─> durable store
Rust agent and CLI ───────────────────┘               └────────> cut-engine
```

In the target architecture, `cutd` is the only canonical owner of project state, revisions, history, idempotency receipts, proposals, and jobs. The current tree has not finished this ownership migration. UI, MCP, agent, and CLI code must eventually use the versioned `cutd` protocol. They must not read or mutate `project.cut.json` as an independent state owner.

Keep FFmpeg, ffprobe, yt-dlp, and ONNX as controlled native sidecars. The migration does not require rewriting codecs or model runtimes in Rust.

## Completed work

The current branch contains the existing TypeScript product plus the first Rust migration unit.

- `crates/cut-core` owns deterministic compilation primitives.
- `crates/cut-engine` owns probe, render, proxy, encoder, and vision execution.
- `crates/cut-runtime` owns the canonical runtime and exposes `RuntimeHandle`.
- `RuntimeHandle` serializes requests through one mailbox actor.
- `cutd` and `cut-runtime` expose the versioned JSONL runtime protocol.
- The Tauri adapter delegates canonical mutations to `RuntimeHandle`.
- Runtime state uses an authoritative atomic checkpoint and compatibility projections.
- A failed durable write does not advance the in-memory project or revision.
- Advisory locking allows stale lock-file reuse while rejecting a live second owner.
- Operation receipts and proposal accept or reject receipts survive restarts.
- Proposal JSON emits the camelCase fields expected by the current GUI and reads legacy snake_case checkpoints.
- Runtime inputs reject empty idempotency keys.
- Runtime duration includes project duration, clip placement, clip duration, speed, and gaps.
- Import rejects missing files and malformed overlays.

The current runtime still uses `serde_json::Value` internally. The current `cutd` transport is stdin and stdout JSONL, not a multi-client socket server.

## Verified commands

These commands passed at handoff:

```bash
cargo fmt -p cut-runtime --check
cargo clippy -p cut-runtime --all-targets -- -D warnings
cargo test -p cut-runtime
cargo test --manifest-path packages/desktop/src-tauri/Cargo.toml --lib --bins
cargo test --workspace
```

`cut-runtime` has 26 passing tests. Run `scripts/smoke-cutd.sh` to verify state read, apply, restart recovery, proposal camelCase output, and empty-key rejection.

`cargo fmt --all --check` is not a clean repository-wide gate yet because pre-existing `cut-engine` files need formatting. Do not rewrite unrelated files merely to hide that distinction.

The desktop build emits an existing macOS linker-version warning. No runtime test failed because of the warning.

## Next implementation plan

Complete the units in this order. End every unit with its own focused tests and real-surface check.

### 1. Make `cutd` the shared process owner

Replace the stdin-only daemon boundary with project-scoped local IPC.

- Keep `cutd` as the canonical daemon binary. Retain `cut-runtime` only as a time-boxed compatibility alias until every launcher calls `cutd`.
- Derive one endpoint from the canonical project-directory path. Use a hashed path under the user's runtime directory when a Unix socket would exceed platform path limits.
- Restrict the endpoint to the current user. Reject connections from other users when the platform exposes peer credentials.
- Let clients connect to an existing daemon or start one and wait for readiness. A disconnected client must reconnect, re-read the revision, and retry only requests with stable idempotency keys.
- Use a Unix domain socket on macOS and Linux.
- Define the Windows named-pipe counterpart before claiming cross-platform completion.
- Keep `RequestEnvelope`, `RuntimeRequest`, `RuntimeResponse`, and `RuntimeError` as the versioned protocol boundary.
- Add client connection, reconnect, timeout, request-size, and cancellation behavior.
- Make the Tauri adapter and TypeScript MCP compatibility client connect to the same running `cutd`.
- Update `packages/mcp/src/runtime-client.ts`, `packages/desktop/src-tauri/src/main.rs`, and the root runtime build script in `package.json`. Remove their in-process or per-client runtime ownership.
- Add an integration test with two simultaneous clients, one revision conflict, one idempotent retry, daemon termination, and restart recovery.
- Prove that closing one client does not stop a daemon that another client or job still uses.

Do not let clients fall back to direct project-file mutation.

### 2. Introduce the typed Rust domain

Create `crates/cut-domain` as a Cargo workspace member and remove `serde_json::Value` from canonical project and command logic.

- Add newtypes for project, source, track, clip, proposal, job, revision, transaction, request, and idempotency identifiers.
- Model clip content, track kinds, overlays, commands, and proposal states as enums.
- Preserve unknown external extension fields at the serialization boundary.
- Port the TypeScript v1-to-v2 migration and alias rules into one Rust wire adapter.
- Add golden fixtures that compare current TypeScript and Rust parse, migration, command, undo, redo, duration, and serialization behavior.
- Move validation and command application out of `cut-runtime` into `cut-domain`.
- Make `crates/cut-runtime` depend on `cut-domain`. Let `crates/cut-engine` accept validated domain values or an explicit wire adapter owned by `cut-domain`.

Do not maintain independent TypeScript and Rust reducers after caller migration.

### 3. Harden durable storage and jobs

Replace unbounded full-snapshot history with a journal and bounded checkpoints.

- Store typed commands and inverse commands.
- Add checksummed records and torn-tail recovery.
- Add Windows-safe atomic replacement and Windows CI coverage.
- Persist render and import job lifecycle or reconcile abandoned jobs on startup.
- Route `packages/mcp/src/host.ts` render operations through `cutd` before removing its TypeScript `RenderJobManager`.
- Do not claim that `cutd` owns jobs until MCP render and desktop render share the same runtime job records.
- Define the pending-proposal mutation policy. Reject, cancel, or explicitly rebase concurrent edits instead of leaving stale proposals accidentally.
- Make MCP validation read authoritative runtime state before compatibility projections.
- Add explicit v1 bootstrap migration.
- Import `project.cut.json` only when no authoritative checkpoint exists or through an explicit import or rebuild command. After checkpoint creation, treat it as a read-only compatibility projection. Detect divergent human edits instead of silently ignoring them.
- Test first launch, v1 migration, explicit projection import, projection divergence, checkpoint recovery, and projection repair.

### 4. Add the native Rust editor shell

Create `crates/cut-app` as the native application crate. Its `cut` binary becomes the packaged desktop product and composition root. Use `winit`, `egui`, `egui-wgpu`, and `wgpu`.

- Start with project name, revision, track list, proposal status, and undo or redo.
- Keep canonical project state in `cutd`. The UI owns only view state.
- Use clear view-model boundaries for viewer, timeline, inspector, agent rail, media bin, and settings.
- Add a real command-surface smoke test before porting more controls.
- Treat Korean IME, keyboard navigation, focus, and AccessKit accessibility as release gates.
- Move window lifecycle, icons, configuration, sidecar discovery, signing inputs, and packaging ownership from `packages/desktop` into `crates/cut-app`.
- Require `cargo build --release -p cut-app` and a native open, edit, restart smoke test before deleting Tauri.

Do not add Slint as a second UI toolkit.

### 5. Add native playback

Create `crates/cut-playback` as a Cargo workspace member behind a stable playback interface. `crates/cut-app` owns its UI adapter. `crates/cut-engine` continues to own offline render and probe behavior.

- Implement open, play, pause, seek, volume, mute, clock events, and frame delivery.
- Use a bounded decode queue and an audio master clock.
- Map timeline time to source time with clip placement, in and out points, and speed.
- Render frames to a wgpu texture without blocking the UI event loop.
- Verify 4K, variable-frame-rate, long-GOP, captions, mute, fullscreen, and audio synchronization.

Use FFmpeg or another mature native media backend. Do not implement codecs from scratch.

### 6. Port the agent, MCP, CLI, and remaining adapters

- Port provider-neutral agent messages, streaming, cancellation, tool calls, and proposal staging to Rust.
- Add Anthropic, OpenAI Chat Completions, and OpenAI Responses adapters behind one Rust trait.
- Port MCP stdio routing and preserve current tool names and profiles.
- Port fetch, skills, interchange, and CLI behavior.
- Keep secrets out of projects, checkpoints, journals, logs, and error payloads.
- Add protocol conformance fixtures before removing each TypeScript caller.

### 7. Delete TypeScript and Tauri

Delete legacy code only after the native application and Rust MCP surface pass the same real workflow:

```text
open project
import media
play and seek
apply inspector edit
send agent prompt
review and accept or reject proposal
render preview
restart
connect MCP as a second client
verify revision, project, history, and receipts
```

Then remove every ported TypeScript package: `packages/gui`, `packages/desktop`, `packages/app`, `packages/agent`, `packages/mcp`, `packages/timeline`, `packages/media`, `packages/engine`, `packages/fetcher`, `packages/interchange`, `packages/vision`, `packages/cli`, `packages/skills`, and `packages/evals`.

Remove the Node workspace files after their Rust replacements land: `package.json`, `package-lock.json`, `pnpm-lock.yaml`, `pnpm-workspace.yaml`, `tsconfig.json`, `tsconfig.base.json`, `tsconfig.tsbuildinfo`, and `vitest.workspace.ts`. Remove generated Tauri schemas and all Vite, React, TypeScript, and Tauri dependencies. Preserve `skills/*/SKILL.md` as data consumed by the Rust skills loader.

Use these final deletion checks:

```bash
if rg --files --hidden -g '!.git' | rg '\.(ts|tsx|js|jsx)$'; then
  echo 'TypeScript or JavaScript files remain' >&2
  exit 1
fi

if rg -n -i 'tauri|react|vite|typescript' Cargo.toml crates; then
  echo 'Legacy production dependencies remain' >&2
  exit 1
fi

cargo test --workspace
```

## Known follow-up decisions

- Decide whether idempotency keys are globally unique across operation kinds or namespaced by operation.
- Decide whether unsupported live effects use a rendered-preview fallback during native playback.
- Set concrete frame-time, command-to-paint, memory, recovery, installer-size, and cold-start budgets.
- Package and sign FFmpeg, ffprobe, yt-dlp, ONNX models, and other sidecars for each target platform.

## Instructions for the next task

Start with unit 1. Inspect the current `cut-runtime` protocol and `packages/mcp/src/runtime-client.ts`. Implement the smallest project-scoped IPC server and client that proves two clients share one `cutd`. Do not begin the native UI until that ownership invariant works end to end.
