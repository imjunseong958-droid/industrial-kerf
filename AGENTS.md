<!-- BEGIN Lim_claude: managed by lim.sh and replaced on update. Put repo-specific rules below the END line. -->
## Order of work

Do each step once, in order. Later rounds look only at what changed since the last one. Never restart a check.

1. Before implementing, write down how you will show the task is done: for each thing it asks, what you will run and what you expect to see.
2. Implement, running only the related tests. Do not commit yet.
3. Verify: `.claude/skills/verify/SKILL.md` (the `verify` skill in Claude Code).
4. Commit. The commit gate blocks it until this session has run the verify skill.
5. Break: start one subagent in the background that tries to make the committed change fail (Subagents). Do not wait for it, and do not touch the code while it runs, or its results go stale.
6. Meanwhile, run the repo's checks once (Verification 2), push, and open the PR.
7. When it reports, fix each failure it reproduced that the change caused, add a test for it, commit, and push. Send the fix diff to the same subagent to rerun its reproductions. Recheck by hand only what a fix changed.
8. Merge without asking once the checks pass and the breaker has reported (Merging).

After the PR is open (feedback, merging the default branch in, conflicts): make the change, run the related tests, and commit with a `Verified:` trailer naming them. No new break or verify run unless the user asks. The commit gate allows these commits, in a new session too, since the branch has a verified commit.

## Verification

1. Done means you ran the change and saw it do what the task asked. Passing checks or unit tests alone is not done.
2. The repo's checks are `.claude/verify.sh`, or else the build, linter, and tests the project uses (README, CI config, Makefile, package manifest). Run them once, on the final code, before opening the PR. Without CI config, the push gate runs them on each `git push`, so do not run them first. Never run a check twice on the same code.
3. Never change tests, snapshots, golden files, lint settings, CI config, or `.claude/verify.sh` to get a pass. Change a test only when the task changes the behavior it checks, and say so in the PR.
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
3. Once verified, commit, push, and open or update the PR without asking. Ask before force-pushing.
4. Keep the PR description short: what changed; the evidence for each item from Order of work 1 (commands run, what you saw); what was not verified and why; tests or config you changed; the breaker's results (failures fixed, failures left unfixed, its unconfirmed list, or that it found nothing).
5. End each commit message with a one-line `Verified:` trailer: the checks you ran and their result, and what you tried by hand (for a fix, how you checked it). If unchecked, write `Verified: none (<reason>)`, e.g. `Verified: none (docs only)`, which the commit gate accepts. Put it in the last paragraph, directly above trailers such as `Co-Authored-By` with no blank line between: git reads only the last paragraph as trailers. Merge commits need none.

## Merging

1. Merge the PRs you opened for a task, and those the user asks you to, without asking. No others.
2. One at a time. First merge the latest default branch into the PR branch, resolve conflicts, and push. Do not rebase: that needs a force-push.
3. Resolve a conflict by keeping what both sides meant. Never drop a side to make it go away. If both changed the same behavior differently and you cannot tell which is right, ask the user.
4. Check the merged result once. With CI on PRs, wait for it and do not also run the checks locally. Without CI, the push gate checks it on that push.
5. If it fails, fix it on the PR branch, commit, and check again (Verification 6).
6. Do not merge, and tell the user instead, if a person requested changes or left unresolved comments, a reproduced failure the change caused is unfixed, the PR deletes a flaky test, or branch protection blocks it. Never bypass branch protection or required checks.
7. Use the repo's merge method (squash, merge commit, or rebase).

## Subagents

1. Spawn none unless the user asks, except one breaker per task (Order of work 5), run in the background.
2. Give it the task, the diff of your commits against the default branch, and how to run the project. Not your reasoning or your verification report. If the repo has a REVIEW.md, it reads the default branch's version.
3. It tries to make the change fail by running it, in at most 15 commands aimed at the inputs most likely to break it, and reports only failures it reproduced: the command, the input, what the task says should happen, and what happened. It runs the default branch's version of each test the diff changes or deletes against the new code and reports failures, and it lists changes the task did not ask for to the other checks in Verification 3. A failure that also happens on the default branch is marked pre-existing. It reports no style, naming, structure, or future needs. Suspicions it could not reproduce, and behavior the task does not decide, go in an "unconfirmed" list.
4. Send it the fix diff to rerun its reproductions (in Claude Code, continue it with SendMessage). If you cannot continue it, start one new breaker with its findings and that diff. No subagents for second opinions.

## Rust

1. Run `cargo check -q` after each batch of changes and fix errors before going on.
2. While working, run only the relevant tests (`cargo test -q <filter>`). The full suite runs once on the final code (Verification 2).
3. Run `cargo fmt` before committing. If `.claude/verify.sh` does not run `cargo clippy --all-targets -q`, run it once on the final code. Fix warnings your change introduced instead of adding `#[allow]`.
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
