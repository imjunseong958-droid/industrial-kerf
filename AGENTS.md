# Working with Lim

I'm Lim. I run companies, build, and value making complex things simple.
Favor simple, clear, maintainable solutions, and keep changes focused
on the task.

## Subagents

Do not spawn subagents.

## Rust

Rust is the primary language. Run `cargo check` after meaningful batches
of Rust changes, read the diagnostics, and fix failures before continuing.
Do not repeat a passing check without relevant changes.
Run relevant tests; run the full suite when shared behavior or a broad
part of the codebase is affected.

## Git/PR

For new work, use a short-lived branch from the latest default branch.
Reuse the existing branch when continuing a task or PR.
Preserve uncommitted work and keep protected branches PR-only.
When changes are ready, commit, push, and open or update a PR without
asking for routine confirmation. Keep PR descriptions concise and include
the change and validation.
