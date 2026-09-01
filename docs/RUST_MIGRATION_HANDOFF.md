# Rust migration handoff

This reference records the completed Rust-native Cut migration. Start a new
Codex task by asking it to read this file before changing the native runtime,
playback, agent, or MCP boundaries.

## Target architecture

The final application removes TypeScript, React, and Tauri from production. The target uses these boundaries:

```text
native editor (winit + egui + wgpu) ─┐
Rust MCP server ──────────────────────┼─> cutd ─> typed domain ─> durable store
Rust agent and CLI ───────────────────┘               └────────> cut-engine
```

In the target architecture, `cutd` is the only canonical owner of project state,
revisions, history, idempotency receipts, proposals, and jobs. All production
callers use the versioned `cutd` protocol. They must not read or mutate
`project.cut.json` as an independent state owner.

Keep FFmpeg, ffprobe, yt-dlp, and ONNX as controlled native sidecars. The migration does not require rewriting codecs or model runtimes in Rust.

## Completed work

Units 1 through 7 are complete. The repository now contains the native Rust
editor, runtime, playback, agent, CLI, MCP, fetch, skills, and interchange
surfaces; the former Node and Tauri product packages have been removed.

- `crates/cut-core` owns deterministic compilation primitives.
- `crates/cut-engine` owns probe, render, proxy, encoder, and vision execution.
- `crates/cut-runtime` owns the canonical runtime and exposes `RuntimeHandle`.
- `RuntimeHandle` serializes requests through one mailbox actor.
- `cutd` owns one project-scoped local IPC endpoint; `cut-runtime` remains a
  time-boxed compatibility alias and stdio adapter.
- The native application delegates canonical mutations through `RuntimeClient`
  to the daemon's `RuntimeHandle`.
- `crates/cut-domain` owns the typed project, command, inverse-command, and
  migration boundary; runtime wire JSON remains an adapter concern.
- Runtime state uses a checksummed typed journal, bounded atomic checkpoints,
  torn-tail recovery, and compatibility projections.
- A failed durable write does not advance the in-memory project or revision.
- Advisory locking allows stale lock-file reuse while rejecting a live second owner.
- Operation receipts and proposal accept or reject receipts survive restarts.
- Render job lifecycle records survive restarts and queued or running jobs are
  reconciled as failed when `cutd` restarts. MCP and desktop use the same
  runtime job records while their local schedulers only execute the work.
- Media imports are synchronous atomic runtime transactions with durable
  idempotency receipts, so they do not leave a partial import job to recover.
- v1 bootstrap, explicit projection import, projection repair, and divergent
  compatibility projection detection are owned by `cutd`.
- Proposal JSON emits the camelCase fields expected by the current GUI and reads legacy snake_case checkpoints.
- Runtime inputs reject empty idempotency keys.
- Runtime duration includes project duration, clip placement, clip duration, speed, and gaps.
- Import rejects missing files and malformed overlays.
- `crates/cut-app` owns the native `winit` + `egui` + `egui-wgpu` + `wgpu`
  composition root, native window configuration, icon, sidecar discovery, and
  packaging guidance.
- The native shell reads and mutates project state only through `RuntimeClient`
  and projects the canonical snapshot through viewer, timeline, inspector,
  agent rail, media bin, and settings view-model boundaries.
- `crates/cut-playback` owns the worker-backed native playback boundary. It
  maps typed timeline clips to FFmpeg decoding, emits bounded clock and RGBA
  frame events, and uses cpal with a monotonic fallback clock when no audio
  device is available.
- `crates/cut-app` owns the playback adapter, egui texture upload, captions,
  viewer controls, and native fullscreen window action. `cut-engine` remains
  the offline render and probe owner.
- `crates/cut-playback/tests/release_gate.rs` generates real 4K, VFR, long-GOP,
  audio/video, and caption media and verifies decode, seek, clock mapping,
  captions, mute state, and bounded frame delivery.
- The native release window was manually verified with AccessKit: open/edit/
  restart persistence, Korean IME paste, focused Tab traversal, mute,
  fullscreen, playback controls, and the agent rail.
- `crates/cut-agent` owns provider-neutral history, streaming, cancellation,
  normalized tool calls, multimodal result limits, and proposal staging.
- `crates/cut-mcp` owns MCP JSON-RPC stdio routing, the core/full tool
  profiles, and the optional inbound stdio registry used by `lcut`.
  `crates/cut-cli` owns `lcut`; `cut-fetcher`, `cut-skills`, and
  `cut-interchange` are shared Rust adapters.
- Anthropic Messages, OpenAI Chat Completions, and OpenAI Responses are
  normalized behind one provider trait. `fixtures/protocol/conformance.json`
  locks the tool and message shapes used at those boundaries.
- The real Rust workflow was exercised with local media, two MCP clients,
  revision conflict and idempotent retry, preview render, undo/redo, daemon
  restart recovery, CLI agent proposal staging, and proposal accept/reject.
- `cut --smoke` and `scripts/smoke-cut-app.sh` exercise real add-track,
  undo, redo, release-build, and daemon-restart behavior.

The runtime wire boundary still uses `serde_json::Value` for compatibility, but
canonical project and command reduction is typed in `cut-domain`. Unix socket
IPC is implemented on macOS and Linux. The Windows named-pipe address is
defined and Windows-safe file replacement is covered by CI, but named-pipe
transport remains unfinished.

## Verified commands

These commands pass on the native tree:

```bash
cargo fmt -p cut-runtime --check
cargo test -p cut-runtime
cargo test --workspace
cargo fmt -p cut-app --check
cargo test -p cut-app
cargo test -p cut-playback --all-targets -- --nocapture
cargo clippy -p cut-agent -p cut-mcp -p cut-cli -p cut-app \
  -p cut-fetcher -p cut-interchange -p cut-skills -p cut-playback \
  -p cut-runtime --all-targets --no-deps -- -D warnings
cargo build --release -p cut-runtime --bin cutd
cargo build --release -p cut-app --bin cut
scripts/smoke-cut-app.sh
```

`cut-runtime` has 32 unit tests and 2 real-process IPC tests. Run
`scripts/smoke-cutd.sh` to verify state read, apply, restart recovery, proposal
camelCase output, and empty-key rejection.

The inbound MCP registry was exercised against a real Rust `cut-mcp` child with
`lcut --mcp`; terminal agents can opt into the same registry with
`--mcp-tools` or `CUT_ENABLE_MCP=1`.

`cargo fmt --all --check` is not a clean repository-wide gate yet because
pre-existing `cut-engine` files need formatting. Do not rewrite unrelated files
merely to hide that distinction.

The desktop build emits an existing macOS linker-version warning. No runtime test failed because of the warning.

## Implementation record

Each unit ended with focused tests and a real-surface check before the next unit
started.

### 1. Make `cutd` the shared process owner (completed)

Status: complete. `cutd` owns a project-scoped Unix IPC endpoint, while
`RuntimeClient` handles readiness, reconnect, request limits, stable-key replay,
and revision-aware failures. Two-client conflict, idempotent retry, client
isolation, daemon termination, and restart recovery are covered by real-process
tests.

### 2. Introduce the typed Rust domain (completed)

Status: complete. `cut-domain` owns typed identifiers, projects, tracks, clips,
overlays, commands, migrations, history, duration, validation, and canonical
wire serialization. The domain golden fixture locks migration, command,
undo/redo, duration, and serialization behavior.

### 3. Harden durable storage and jobs (completed)

Status: complete. `cut-runtime` persists checksummed typed journal entries,
bounded checkpoints, receipts, proposals, and render jobs; it recovers torn
tails, repairs compatibility projections, rejects divergent projections, and
reconciles abandoned jobs after restart. Proposal accept/reject and media
import are durable, revisioned operations.

### 4. Add the native Rust editor shell (completed)

Status: complete. The native shell, runtime-backed view-models,
command-surface smoke, release build, daemon-restart smoke, Korean IME,
keyboard navigation, focus, AccessKit, and open-edit-restart workflow passed.

`crates/cut-app` is the native application crate. Its `cut` binary is the
packaged desktop product and composition root built on `winit`, `egui`,
`egui-wgpu`, and `wgpu`.

- Start with project name, revision, track list, proposal status, and undo or redo.
- Keep canonical project state in `cutd`. The UI owns only view state.
- Use clear view-model boundaries for viewer, timeline, inspector, agent rail, media bin, and settings.
- The real command-surface smoke test covers add-track, undo, redo, and daemon
  restart.
- Korean IME, keyboard navigation, focus, and AccessKit accessibility passed
  as release gates.
- Window lifecycle, icons, configuration, sidecar discovery, and packaging are
  owned by `crates/cut-app`.
- `cargo build --release -p cut-app` and the native open/edit/restart smoke test
  passed before legacy deletion.

Do not add Slint as a second UI toolkit.

### 5. Add native playback (completed)

Status: complete. `cut-playback` exposes a bounded worker-backed session with
typed timeline mapping, FFmpeg RGBA frame delivery, clock events, cpal audio
output with a monotonic fallback, and control commands. `cut-app` owns the
adapter, texture upload, captions, viewer controls, and fullscreen window
action. The release-gate integration tests and native manual checks passed.

`crates/cut-playback` is a Cargo workspace member behind a stable playback
interface. `crates/cut-app` owns its UI adapter and `crates/cut-engine` owns
offline render and probe behavior.

- Open, play, pause, seek, volume, mute, clock events, and frame delivery are
  implemented.
- Decode uses a bounded queue and an audio/monotonic master clock.
- Timeline mapping handles placement, in/out points, and speed.
- Frames are rendered to a wgpu texture without blocking the UI event loop.
- 4K, variable-frame-rate, long-GOP, captions, mute, fullscreen, and audio
  synchronization were verified.

Use FFmpeg or another mature native media backend. Do not implement codecs from scratch.

### 6. Port the agent, MCP, CLI, and remaining adapters (completed)

Status: complete. The Rust workflow is stable on the actual cutd/MCP/CLI
surface; API credentials remain user-owned and are not part of the test tree.

- Port provider-neutral agent messages, streaming, cancellation, tool calls, and proposal staging to Rust.
- Add Anthropic, OpenAI Chat Completions, and OpenAI Responses adapters behind one Rust trait.
- Port MCP stdio routing, preserve current tool names and profiles, and route
  configured inbound servers through `mcp_search` and `mcp_call`.
- Port fetch, skills, interchange, and CLI behavior.
- Keep secrets out of projects, checkpoints, journals, logs, and error payloads.
- The shared JSON conformance fixture is tested by the Rust provider and MCP
  surfaces before legacy callers are removed.

### 7. Delete legacy Node and Tauri surfaces (completed)

Status: complete. The native workflow passed before the legacy packages and
workspace files were removed.

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

The ported packages were removed: `packages/gui`, `packages/desktop`,
`packages/app`, `packages/agent`, `packages/mcp`, `packages/timeline`,
`packages/media`, `packages/engine`, `packages/fetcher`,
`packages/interchange`, `packages/vision`, `packages/cli`, `packages/skills`,
and `packages/evals`.

The Node workspace files and generated desktop schemas were removed after the
Rust replacements landed. Preserve `skills/*/SKILL.md` as data consumed by the
Rust skills loader.

Use these final deletion checks:

```bash
if rg --files --hidden -g '!.git' | rg '\.(ts|tsx|js|jsx)$'; then
  echo 'Legacy script files remain' >&2
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

Keep `cutd` as the sole project-state owner. Extend the Rust protocol and
focused tests together; do not reintroduce a second state reducer, a second UI
toolkit, or a Node/Tauri compatibility layer.
