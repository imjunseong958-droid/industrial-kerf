---
name: verify
description: Check that a code change actually works by running it, not just by reading it. Run once per task, before the first commit. Not for review fixes, merges, rebases, conflict resolution, updates to an open PR, or doc-only changes.
---

# Verify

Goal: evidence that the change does what the task asked, gathered by running it. This is step 2 of the Order of work in AGENTS.md: once per task, before the first commit. The commit gate blocks commits in a session that has not run this skill.

## When not to run it

1. It already ran in this task. For review fixes, use by hand only the behavior a fix changed (AGENTS.md Order of work 5).
2. Merging, rebasing, cherry-picking, or resolving conflicts between branches or PRs. AGENTS.md Merging says how the merged result is checked.
3. Updating an open PR (feedback, a fix after checks failed). Run the tests for the fix. CI or the push gate covers the rest.
4. The change touches only docs, comments, or the PR description. Commit it with `Verified: none (docs only)`.

## 1. Checks

1. Run the tests related to the change.
2. Do not run `.claude/verify.sh` or the full suite here. They run once on the final code, before the PR is opened (AGENTS.md Order of work 6).

## 2. Use the change the way a user would

For a refactor meant to keep behavior, compare the old and new output on the same inputs instead (AGENTS.md Tests 8).

Run the affected entry point once with realistic input. Add one bad input only when the change is about handling input.

1. CLI or binary: run it with real arguments and input.
2. Library: call the public API from a small scratch program, or from a test only if it is worth keeping under the Tests rules in AGENTS.md.
3. HTTP service: start it locally and call the changed endpoints.
4. Web UI: open it in a browser, go through the changed flow, and take a screenshot.
5. Mobile or desktop app: run it in the simulator or the app and go through the flow.

If the repo's CLAUDE.md or README says how to run it, follow that.
Compare the result with what the task asked for, not with what the code appears to do.

## 3. Rules

1. Do not change tests, snapshots or golden files, lint settings, CI config, or `.claude/verify.sh` to get a pass.
2. If a failure is flaky, do not rerun until green, add sleeps, or skip the test. Find the cause. If you can't, delete the test and write down why (AGENTS.md Verification 5).
3. After 3 failed fix attempts on the same check, stop and report.
4. Delete scratch files, including tests you wrote only to check this change once, and stop any servers you started.

## 4. Report

Write down what you ran and what you saw:

1. Commands and their result (pass or fail, key output lines)
2. What you tried by hand and what happened (screenshots for UI)
3. What you did not verify, and why

Put this report in the PR description or your reply, not in a file in the repo, and sum it up in the commit's `Verified:` trailer (AGENTS.md Git and PRs 5).
