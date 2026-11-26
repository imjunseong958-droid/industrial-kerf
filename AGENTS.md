<!-- BEGIN Lim_claude: managed by lim.sh and replaced on update. Put repo-specific rules below the END line. -->
## Verification

1. Done means you ran the change and saw it work. Passing checks or unit tests alone is not done.
2. Before committing, follow `.claude/skills/verify/SKILL.md`. In Claude Code this is the `verify` skill. If `.claude/verify.sh` exists, it holds this repo's checks.
3. Do not change tests, snapshots or golden files, lint settings, CI config, or `.claude/verify.sh` to make your change pass. Change a test only when the task changes the behavior it checks, and say so in the PR.
4. Fix warnings your change introduces instead of silencing them.
5. Flaky tests: do not rerun until green, add sleeps, raise timeouts, or skip them. Find the cause. If you can't, stop and tell the user.
6. If a check still fails after 3 fix attempts, stop and report what fails, what you tried, and your best guess at the cause.
7. Do not rerun a passing check without relevant changes.

## Tests

1. Add a test only for behavior the task adds or changes, or to reproduce a bug you fixed. Do not add tests to raise coverage or to show work.
2. A test you add must be able to fail. Break the behavior it checks (undo the fix or change the code under test), see the test fail, then restore the code. Delete a test that passes either way.
3. Cover new or changed behavior with an end-to-end or golden test through the public interface (CLI, HTTP endpoint, public API) where practical, extending the existing suite. Use unit tests only for logic that is hard to reach that way.
4. When the task changes golden or snapshot output on purpose, regenerate it, read the diff, and confirm every changed line is intended. List the changed golden files in the PR.
5. Do not write tests that mirror the implementation, only check mocks you set up, check that a constructor stores its arguments, cover trivial getters or setters, or test the language, a framework, or a library.
6. Add cases to existing test files before creating new ones. Do not repeat a case an existing test already covers.
7. To check a change once, use a scratch command or program and delete it afterwards. Do not keep it as a test.
8. For parsing, encoding, or state logic, add property tests (round-trip, invariants, idempotency) where practical. For refactors meant to keep behavior, compare old and new output on the same inputs instead of adding tests.

## Docs

1. Do not create new doc files (architecture overviews, design notes, summaries, plans, reports) unless the user asks for one.
2. Write down only what the code cannot show: how to install, run, or use the project, why a non-obvious choice was made, and constraints from outside the code. Do not describe directory layout, what each module or file does, call flows, or lists of types and functions, in READMEs, AGENTS.md, or anywhere else. Agents read the code faster than such docs, and the docs go stale and mislead them.
3. Put plans, progress notes, and verification reports in your reply or the PR description, not in files in the repo.
4. When your change makes an existing doc wrong (usage, options, setup steps), fix that part in the same change.

## Git and PRs

1. For new work, use a short-lived branch from the latest default branch. Reuse the existing branch when continuing a task or PR. Never push directly to the default branch.
2. Never discard uncommitted work. Commit only the changes made for the task.
3. Once verification passes, commit, push, and open or update a PR without asking. Ask before force-pushing or merging.
4. Keep PR descriptions concise: what changed, and how you verified it (commands run, what you saw). If something was not verified, say so and why. Call out any test or config you changed.

## Subagents

1. Do not spawn subagents unless the user asks.
2. Exception: after the final commit and before opening or updating a PR, spawn one reviewer subagent. Give it the task and the diff against the default branch, not your reasoning or conclusions. Ask it to look for bugs and for tests or checks that were weakened. If the repo has a REVIEW.md, have it read the version on the default branch. Fix the findings you can confirm and list the rest in the PR. Use one reviewer, not several.

## Rust

1. Run `cargo check -q` after meaningful batches of changes and fix errors before continuing.
2. Run relevant tests with `cargo test -q`. Run the full suite when shared behavior or a broad area is affected.
3. Before committing, run `cargo fmt` and `cargo clippy --all-targets -q`. Fix warnings your change introduced instead of silencing them with `#[allow]`.
4. Put behavior tests in `tests/` against the public API or the built binary. Use `proptest` for property tests.
5. Do not add `#[ignore]` to get past a failing test.
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
