# Working with Lim

I'm Lim. I run companies, build, and value making complex things simple.
Make the smallest coherent change in the existing style.
Include only changes needed to complete the request.
No overbuilding, unrelated features, cleanup, or fixes.

## Subagents

Do not spawn subagents. Do the work in this session.

## Rust

Rust is the primary language. Use the compiler as an assistant.

After each small, coherent change, run `cargo check`.
Read the diagnostics, not the whole log. Fix failures and check again.
Once it passes, do not repeat it without new relevant changes.
Do not defer compile errors until the end of a large edit.
Ask when progress stalls or a decision requires user input.

Write tests for meaningful behavior and realistic failures.
Run relevant tests after the change that should make them pass.
Run the full suite once at the end when shared code or multiple modules
are affected. When checks pass, stop unless something new shows up.

## Git/PR

Trunk-based. Short-lived branch, then PR, merge, delete.
Unmerged branches are never deleted.

Protected: main, master, develop, release/*, staging.
Do not commit to or push those, or edit someone else's branch.
If starting new work on a protected branch, create a new branch first.
After a PR merges, fast-forward the local default branch to its remote
and switch to it. This local sync does not count as editing a protected
branch; do not leave HEAD detached or the local default branch behind.
Only fast-forward when the worktree is clean and the local branch is an
ancestor of its remote. Otherwise preserve the local state and report why.

New branch off latest default: `lim/<feat|fix|chore>-<desc>`.
Do not reuse an old branch for new work.

Commit, push, open/close PR, merge, and delete only when asked.
No `git add .`, secrets, `--no-verify`, or foreign force-push.

When asked for a PR: default base, draft; update this branch's PR
if it exists. Do not list other PRs, fetch CI, or list worktrees.
Merge only if asked.
