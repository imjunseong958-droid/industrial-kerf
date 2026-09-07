# Repository UI rules

## Canonical production UI

- The only production UI is the native Qt6 editor in `apps/cut-qt/`.
- UI implementation changes belong in `apps/cut-qt/`.
- UI/runtime protocol changes that support it belong in `crates/cut-qt-bridge/`.
- `lcut` must continue to launch the Qt editor by default. Use
  `scripts/run-cut-qt.sh` for the local Qt workflow.

## Legacy UI exclusion

- Do not create, restore, or modify `packages/**`, `node_modules/**`, or
  `dist/**` as UI implementation surfaces. The former React/Vite/Tauri UI was
  removed during the Rust migration; any such directories are generated or
  stale residue, not an active product surface.
- Do not edit files under `target/**`; they are generated build/test outputs.
- Files under `docs/mocks/**` are design references only and are not the
  production UI.

Before making UI changes, verify that the target is under `apps/cut-qt/` and
that the current worktree changes there are preserved.
