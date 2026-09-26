# Working with Lim

I'm Lim. I run companies, build, and value making complex things simple.
Make the smallest coherent change in the existing style.
Include only changes needed to complete the request.
No overbuilding, and no unrelated features, cleanup, or fixes.

## Subagents

Do not spawn subagents. Do the work in this session.

## Rust

Rust is the primary language. Use the compiler as an assistant.

After each small, coherent Rust change, run `cargo check`.
Read the diagnostics, not the whole log. Fix failures and check again.
Once it passes, do not repeat it without new relevant changes.
Do not defer compile errors until the end of a large edit.
Ask when progress stalls or a decision requires user input.

Testing rules are only in Tests. `cargo check` does not compile tests.

## Tests

Test by risk. Use the cheapest test that catches the failure.

* Pure logic where bugs are costly: table-driven golden tests. List failure modes first; each becomes a case.
* Cross-module features: one E2E test that writes a small, repeatable artifact. Read the artifact, not the log.
* External services: read-only smoke tests, skipped by default. Run only when asked.
* Glue and refactors: no new tests.

A bug fix adds one regression case at the lowest level.
No tests that restate the implementation.

Run only the affected tests and read only failures.
Run a changed test once; rerun it only after a fix.
Before a PR, run the full suite once.

## Git/PR

Trunk-based. Lifecycle: short-lived branch, then PR, merge, delete.
Commit, push, open/close PR, merge, and delete only when asked.
Never delete an unmerged branch, even if asked.

Protected: main, master, develop, release/*, staging.
Do not commit to or push those, or edit someone else's branch.
If starting new work on a protected branch, branch off the latest default first.
After a PR merges, fast-forward the local default branch to its remote
and switch to it. This local sync does not count as editing a protected
branch; do not leave HEAD detached or the local default branch behind.
Only fast-forward when the worktree is clean and the local branch is an ancestor of its remote. Otherwise preserve the local state and report why.

New branch off latest default: `lim/<feat|fix|chore>-<desc>`.
Do not reuse an old branch for new work.

No `git add .`, secrets, `--no-verify`, or foreign force-push.

When asked for a PR: default base, draft; update this branch's PR
if it exists. Do not list other PRs, fetch CI, or list worktrees.
