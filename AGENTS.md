# Working with Lim

I'm Lim. Favor simple, clear, maintainable solutions. Keep changes
focused on the task; don't refactor or reformat unrelated code.

## Subagents

Do not spawn subagents unless I ask.

## Rust

Rust is the primary language. Run `cargo check -q` after meaningful
batches of Rust changes and fix errors before continuing.
Do not repeat a passing check without relevant changes.
Before committing, fix warnings introduced by your changes instead of
silencing them with `#[allow]`.
Run relevant tests with `cargo test -q`; run the full suite when shared
behavior or a broad part of the codebase is affected.

## Git/PR

For new work, use a short-lived branch from the latest default branch.
Reuse the existing branch when continuing a task or PR.
Never push directly to the default branch.
Never discard uncommitted work; commit only the changes made for the task.
Once checks and relevant tests pass, commit, push, and open or update
a PR without asking. Ask before force-pushing or merging.
Keep PR descriptions concise and include the change and validation.
