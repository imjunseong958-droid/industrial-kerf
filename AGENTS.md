<!-- BEGIN Lim_claude: managed by lim.sh and replaced on update. Put repo-specific rules below the END line. -->
## Verification

1. Done means you ran the change and saw it work. Passing checks or unit tests alone is not done.
2. Before committing, follow `.claude/skills/verify/SKILL.md`. In Claude Code this is the `verify` skill. If `.claude/verify.sh` exists, it holds this repo's checks.
3. Prefer tests that go through the public interface (integration or end-to-end) over tests of internals.
4. For parsing, encoding, or state logic, add property tests (round-trip, invariants, idempotency) where practical. For refactors meant to keep behavior, compare old and new output on the same inputs.
5. Do not change tests, snapshots, lint settings, CI config, or `.claude/verify.sh` to make your change pass. Change a test only when the task changes the behavior it checks, and say so in the PR.
6. Fix warnings your change introduces instead of silencing them.
7. Flaky tests: do not rerun until green, add sleeps, raise timeouts, or skip them. Find the cause. If you can't, stop and tell the user.
8. If a check still fails after 3 fix attempts, stop and report what fails, what you tried, and your best guess at the cause.
9. Do not rerun a passing check without relevant changes.

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

## Motion UI

Before adding or changing UI animation in `apps/kerf-qt`, read
`apps/kerf-qt/MOTION_UI.md`. Keep its scenes table current in the same change,
and put new durations and distances in `tokens::motion`.
