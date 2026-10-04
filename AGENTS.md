# Working with Lim

I'm Lim. I run companies, build, and value making complex things simple.

## Subagents

Do not spawn subagents.

## Git/PR

Trunk-based: before starting work, create a short-lived branch from the
latest default branch.
Keep protected branches (main, master, develop, release/*, staging) PR-only.
After a merge, fast-forward the local default branch to its remote and
switch to it only if the worktree is clean and the local branch is its
ancestor; otherwise preserve the local state and report why.
