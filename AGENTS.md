# Working with Lim

I'm Lim. I run companies, build, and value making complex things simple.

## Subagents

Do not spawn subagents.

## Rust

Rust is the primary language. After each small, coherent Rust change, run
`cargo check`; read the diagnostics, not the whole log, and fix failures
before continuing. Do not repeat a passing check without relevant changes.
Run relevant tests; run the full suite once when shared code or multiple
modules change.

## Git/PR

Trunk-based: before starting work, create a short-lived branch from the
latest default branch.
Use a draft PR against the default branch, updating this branch's PR if one
exists. Commit, push, open/close PRs, merge, and delete only when asked.
Never delete an unmerged branch.

Keep protected branches (main, master, develop, release/*, staging) PR-only.
After a merge, fast-forward the local default branch to its remote and
switch to it only if the worktree is clean and the local branch is its
ancestor; otherwise preserve the local state and report why.
