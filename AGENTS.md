<!-- BEGIN Lim_claude: managed by lim.sh and replaced on update. Put repo-specific rules below the END line. -->
## Order of work

1. Implement, running the tests related to the change as you go.
2. Run the change once (Verification 1).
3. Commit, push, and open the PR.
4. Merge it (Merging).

Fixes and updates after the PR is open: make the change, run the related tests, commit, and push.

## Verification

1. Done means you ran the change once the way a user would (the CLI with real arguments, the changed endpoint, the changed page) and saw it do what the task asked. Passing unit tests alone is not done.
2. With CI, do not run the full suite, build, or linter locally: CI runs them on the PR. Without CI, run them once on the final code before pushing.
3. Never change tests, snapshots, golden files, lint settings, or CI config to get a pass. Change a test only when the task changes the behavior it checks, and say so in the PR.
4. Fix warnings your change introduces. Do not silence them.
5. Flaky test: no rerunning until green, sleeps, longer timeouts, or skips. Find the cause. If you can't, delete the test, say which and why in the PR, and do not merge that PR yourself.
6. If a check still fails after 3 fix attempts, stop and report what fails, what you tried, and your best guess why.

## Tests

1. Add a test only for behavior the task adds or changes, or to reproduce a bug you fixed. Not for coverage or to show work.
2. A new test must be able to fail: break the behavior (undo the fix or change the code under test), see it fail, restore. Delete it if it passes either way.
3. Cover new or changed behavior with end-to-end or golden tests through the public interface (CLI, HTTP endpoint, public API), extending the existing suite, where practical. Unit tests only for logic hard to reach that way.
4. When golden or snapshot output changes on purpose, regenerate it, confirm every changed line is intended, and list the files in the PR.
5. No tests that mirror the implementation, check only your own mocks, check that a constructor stores its arguments, cover trivial getters or setters, or test the language, a framework, or a library.
6. Extend existing test files before adding new ones. Do not repeat a case already covered.
7. One-off checks are scratch commands or programs, deleted afterwards, not tests.
8. Parsing and encoding: property tests (round-trip, idempotency) where practical. Refactors and optimizations meant to keep behavior: compare old and new (or naive and optimized) output on the same inputs instead of adding tests. Normalize differences that don't matter, such as whitespace, and list intended ones in the PR.
9. State machines, concurrency, permission checks, and anything that must never happen (data loss, a double charge, a skipped check): state each invariant in one sentence in the PR and check it mechanically. Enumerate every combination when inputs are few. Otherwise run random operation sequences against a simple reference model (a stateful property test). Use a model checker only if the repo already has one.

## Docs

1. No new doc files (overviews, design notes, summaries, plans, reports) unless the user asks.
2. Write only what the code cannot show: how to install, run, and use the project, why a non-obvious choice was made, and constraints from outside the code. Never directory layouts, per-file descriptions, call flows, or lists of types and functions. Agents read the code faster, and such docs go stale and mislead them.
3. Plans, progress notes, and verification reports go in your reply or the PR, not in repo files.
4. When your change makes part of a doc wrong (usage, options, setup), fix it in the same change.

## Git and PRs

1. New work: a short-lived branch from the latest default branch. Continuing a task or PR: its branch. Never push to the default branch.
2. Never discard uncommitted work. Commit only the task's changes.
3. Commit, push, and open or update the PR without asking. Ask before force-pushing.
4. Keep the PR description short: what changed, what you ran to check it and what you saw, what you did not check and why, and any tests or config you changed.

## Merging

1. Merge the PRs you opened for a task, and those the user asks you to, without asking. No others.
2. Merge once CI on the PR has passed (without CI, once the PR is open). If the default branch moved on since, do not merge it into the PR branch or wait for CI again: merge as long as the host shows no conflicts.
3. On a conflict, merge the latest default branch into the PR branch (do not rebase: that needs a force-push), keep what both sides meant, run the tests related to the conflicting files, push, and merge. Never drop a side to make a conflict go away. If both changed the same behavior differently and you cannot tell which is right, ask the user.
4. If the host will not merge until required checks pass on the new commit, or until the branch is up to date, do what it asks, then merge. Never bypass branch protection or required checks.
5. If CI fails, fix it on the PR branch and push (Verification 6).
6. Do not merge, and tell the user instead, if a person requested changes or left unresolved comments, or the PR deletes a flaky test.
7. Use the repo's merge method (squash, merge commit, or rebase).

## Subagents

Spawn none unless the user asks.

## Rust

1. Run `cargo check -q` after each batch of changes and fix errors before going on.
2. While working, run only the relevant tests (`cargo test -q <filter>`). Leave the full suite to CI (Verification 2).
3. Run `cargo fmt` before committing. Fix clippy warnings your change introduced instead of adding `#[allow]`.
4. Put behavior tests in `tests/` against the public API or the built binary. Use `proptest` for property tests.
5. Make states that must never happen unrepresentable: enums for states and newtypes for validated values, instead of flags and runtime checks.
6. Do not add `#[ignore]` to get past a failing test.
<!-- END Lim_claude -->

## Editor

`apps/kerf-editor` is a Cargo workspace of its own (its manifest says why), so the
workspace-wide commands above leave it out: check, test and lint it with
`--manifest-path apps/kerf-editor/Cargo.toml --workspace`. Its root is a package as well,
so without `--workspace` they cover only that package, not the editor's crates.

## Home folder

Find a person's home folder with `kerf_paths::home()`, their `~/.kerf` with
`kerf_paths::user_dir()`, Kerf's cache folder with `kerf_paths::cache_dir()` and its crash
folder with `kerf_paths::crash_dir()`, never by reading `HOME`: Windows sets none, so code
that reads it finds nothing there. A font folder that each system places by its own rule
may name `HOME` in its macOS and Linux branches.

## Motion UI

Before adding or changing UI animation in `apps/kerf-editor`, read
`apps/kerf-editor/MOTION_UI.md`. Keep its scenes table current in the same change,
and put new durations and distances in `theme::motion` (`apps/kerf-editor/crates/editor_ui/src/theme.rs`).
