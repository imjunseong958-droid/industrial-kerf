---
name: verify
description: Check that a code change works by running it, not by reading it. Once per task, before the first commit. Not for fixes after the break round, merges, rebases, conflict resolution, updates to an open PR, or changes to docs or comments only.
---

# Verify

Goal: evidence, gathered by running the change, for each item planned in AGENTS.md Order of work 1. Once per task, before the first commit. The commit gate blocks commits until this has run.

Skip it if it already ran in this task; for merges, rebases, cherry-picks, and conflict resolution (AGENTS.md Merging); for updates to an open PR (run the fix's tests); and for changes to docs or comments only (commit with `Verified: none (docs only)`).

## 1. Run

1. Run the tests related to the change. Not `.claude/verify.sh` or the full suite: they run once on the final code (AGENTS.md Verification 2).
2. Use the change once the way a user would, with realistic input, following the README if it says how to run it. Add a bad input only when the change handles input.
   - CLI: run it with real arguments and input.
   - Library: call the public API from a scratch program, or from a test if the Tests rules say to keep one.
   - HTTP service: start it and call the changed endpoints.
   - Web UI: go through the changed flow in a browser and take screenshots.
   - Mobile or desktop app: go through the flow in the simulator or the app.
3. A refactor meant to keep behavior: compare old and new output on the same inputs instead.
4. If the change depends on where it runs (OS, shell, paths, worktrees, environment variables), run it in each setting it must support, or list the ones you did not.
5. If the change has invariants (AGENTS.md Tests 9), run their check.
6. Compare each result with what the task asked for, not with what the code appears to do.

## 2. Rules

AGENTS.md Verification 3, 5, and 6 apply: do not change the checks to get a pass, do not rerun a flaky test until green, and stop after 3 failed fixes. Delete scratch files, one-off tests included, and stop any servers you started.

## 3. Report

For each planned item, what you ran and what you saw: commands, key output lines, screenshots. Then what you did not verify, and why. Put it in the PR description or your reply, not in a repo file, and sum it up in the commit's `Verified:` trailer.
