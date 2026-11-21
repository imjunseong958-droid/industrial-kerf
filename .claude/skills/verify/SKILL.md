---
name: verify
description: Check that a code change actually works by running it, not just by reading it or seeing unit tests pass. Use before committing, before opening or updating a PR, and whenever you are about to say a change is done.
---

# Verify

Goal: evidence that the change does what the task asked, gathered by running it.

## 1. Run the project's checks

1. If `.claude/verify.sh` exists, run it. It is the project's own list of checks.
2. Otherwise run the build, linter, and tests the project already uses. Find them in the README, CI config, Makefile, or package manifest.
3. Run the narrowest relevant tests first. Run wider ones if the change touches shared code.

## 2. Use the change the way a user would

Find the entry point the change affects and use it with realistic input:

1. CLI or binary: run it with real arguments and input, including one bad input.
2. Library: call the public API from a small scratch program, or from a test only if it is worth keeping under the Tests rules in AGENTS.md.
3. HTTP service: start it locally and call the changed endpoints.
4. Web UI: open it in a browser, go through the changed flow, and take a screenshot.
5. Mobile or desktop app: run it in the simulator or the app and go through the flow.

If the repo's CLAUDE.md or README says how to run it, follow that.
Compare the result with what the task asked for, not with what the code appears to do.

## 3. Rules

1. Do not change tests, snapshots or golden files, lint settings, CI config, or `.claude/verify.sh` to get a pass.
2. If a failure is flaky, do not rerun until green, add sleeps, or skip the test. Find the cause or report it.
3. After 3 failed fix attempts on the same check, stop and report.
4. Delete scratch files, including tests you wrote only to check this change once, and stop any servers you started.

## 4. Report

Write down what you ran and what you saw:

1. Commands and their result (pass or fail, key output lines)
2. What you tried by hand and what happened (screenshots for UI)
3. What you did not verify, and why

Put this report in the PR description or your reply, not in a file in the repo.
