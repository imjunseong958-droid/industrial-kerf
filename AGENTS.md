<!-- BEGIN Lim_claude: managed by lim.sh and replaced on update. Put repo-specific rules below the END line. -->
## Order of work

Do the steps in this order. Each later round builds on the one before it and looks only at what changed since then. Never start a check over from scratch.

1. Implement. While working, run only the tests related to the change. Commit in step 3, not before.
2. Verify: follow `.claude/skills/verify/SKILL.md` once. In Claude Code this is the `verify` skill.
3. Commit. The commit gate blocks the commit if this session has not run the verify skill.
4. Review round 1: spawn one reviewer subagent on the committed change (Subagents 2). Do not change the code while it reviews. A review of an out-of-date commit is worthless.
5. Fix the findings you can confirm and commit the fixes. Then run another round on the fix commits only (Subagents 3). Use by hand again only the behavior a fix changed, not the whole change. Stop when a round finds nothing you need to fix, or after 3 rounds, and list what is left in the PR.
6. Run this repo's checks once on the final code (Verification 2). In a repo without CI config, skip this step: the push gate runs them in step 7.
7. Push and open the PR (Git and PRs 3 and 4).
8. Merge it without asking (Merging).

Updates after the PR is open (feedback, merging the default branch in, conflict resolution): make the change, run the related tests, and commit with a `Verified:` trailer that names them. Do not review or run the verify skill again unless the user asks. The commit gate lets these commits through, also in a new session, because the branch already has a verified commit.

## Verification

1. Done means you ran the change and saw it work. Passing checks or unit tests alone is not done.
2. This repo's checks are `.claude/verify.sh` if it exists, otherwise the build, linter, and tests the project uses (see the README, CI config, Makefile, or package manifest). Run them once before opening the PR, not before each commit. In a repo without CI config, the push gate runs them on every `git push` and blocks the push if they fail, so do not run them yourself first. While working, run only the tests related to the change. Do not run the same check twice on the same code.
3. Do not change tests, snapshots or golden files, lint settings, CI config, or `.claude/verify.sh` to make your change pass. Change a test only when the task changes the behavior it checks, and say so in the PR.
4. Fix warnings your change introduces instead of silencing them.
5. Flaky tests: do not rerun until green, add sleeps, raise timeouts, or skip them. Find the cause. If you can't, delete the test and say in the PR which test you deleted and why. Do not merge that PR yourself (Merging 6).
6. If a check still fails after 3 fix attempts, stop and report what fails, what you tried, and your best guess at the cause.

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
3. Once verification passes, commit, push, and open or update a PR without asking. Ask before force-pushing.
4. Keep PR descriptions concise: what changed, and how you verified it (commands run, what you saw). If something was not verified, say so and why. Call out any test or config you changed, and list the review findings you did not fix.
5. End every commit message you write with a `Verified:` trailer that says in one line how that commit was checked: the checks you ran and their result, and what you tried by hand. For a review fix, say how you checked the fix. If you did not check it, write `Verified: none (<reason>)`, for example `Verified: none (docs only)`. The commit gate lets such a commit through even if the session has not run the verify skill. Put it in the last paragraph of the message, directly above any other trailers such as `Co-Authored-By`, with no blank line between them. Git reads only the last paragraph as trailers. Merge commits do not need one.

## Merging

1. Merge the PRs you opened for a task, and the PRs the user asks you to merge, without asking. Do not merge other PRs on your own.
2. Merge one PR at a time. Before each, merge the latest default branch into the PR branch, resolve any conflicts, and push. Do not rebase it, since that needs a force-push.
3. Resolve a conflict by keeping what both sides meant to do. Do not drop either side's change to make the conflict go away. If both sides changed the same behavior in different ways and you cannot tell which is right, stop and ask the user.
4. Check the result once. If the repo runs CI on PRs, wait for it to pass and do not also run the checks locally. Without CI, the push gate checks the merged code when you push it in step 2.
5. If the checks fail, fix the code on the PR branch, commit, and check again. Verification 6 applies.
6. Do not merge, and tell the user instead, if a person requested changes or left unresolved comments, if you left a review finding unfixed that may be a real bug, if the PR deletes a flaky test, or if branch protection blocks the merge. Never bypass branch protection or required checks.
7. Use the merge method the repo already uses (squash, merge commit, or rebase).

## Subagents

1. Do not spawn subagents unless the user asks.
2. Exception: one reviewer per task, for the review rounds in Order of work. In round 1, give it the task and the diff of your commits against the default branch, not your reasoning or conclusions. Ask it to look for bugs and for tests or checks that were weakened. If the repo has a REVIEW.md, have it read the version on the default branch.
3. Send later rounds to the same reviewer so it keeps its context (in Claude Code, continue it with SendMessage). Give it only the diff since the commit it last reviewed, and ask whether its findings are fixed and whether the fixes broke anything. If you cannot continue it, start one new reviewer with the earlier findings and that diff. Do not add reviewers for a second opinion, and do not review again after the PR is open unless the user asks.

## Rust

1. Run `cargo check -q` after meaningful batches of changes and fix errors before continuing.
2. While working, run only the relevant tests (`cargo test -q <filter>`). Run the full suite once before opening the PR.
3. Before committing, run `cargo fmt`. Run `cargo clippy --all-targets -q` with the full suite before opening the PR, unless `.claude/verify.sh` already runs it. Fix warnings your change introduced instead of silencing them with `#[allow]`.
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
