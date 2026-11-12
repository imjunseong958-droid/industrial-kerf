#!/usr/bin/env python3
"""Run the editor protocol tests over and over and say how each failure went.

usage: meter.py NAME RUNS [stats] [-- test arguments]

Each run's whole output is kept in logs/NAME-<run>.log. A failing run's
audio lines are printed, the first three in full; the last line counts the
failures.
"""
import os
import re
import subprocess
import sys
import time

name, runs = sys.argv[1], int(sys.argv[2])
rest = sys.argv[3:]
stats = bool(rest) and rest[0] == "stats"
if stats:
    rest = rest[1:]
extra = rest[1:] if rest and rest[0] == "--" else rest
os.makedirs("logs", exist_ok=True)
environment = dict(os.environ)
environment.pop("KERF_PLAYBACK_STATS", None)
if stats:
    environment["KERF_PLAYBACK_STATS"] = "1"
command = ["cargo", "test", "-q", "-p", "kerf-qt-bridge", "--test", "protocol", "--", "--nocapture", *extra]
failed = []
took = []
aside = back = 0
for run in range(1, runs + 1):
    started = time.time()
    result = subprocess.run(command, env=environment, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    took.append(time.time() - started)
    path = f"logs/{name}-{run}.log"
    with open(path, "w") as log:
        log.write(result.stdout)
    aside += "keeping time without it" in result.stdout
    back += "keeping time by it" in result.stdout
    if result.returncode == 0:
        continue
    failed.append(run)
    lines = result.stdout.splitlines()
    kept = [line for line in lines if re.match(r"playback audio|kerf playback|thread '|test .* (ok|FAILED)$", line)]
    panics = [line for line in lines if " readings in " in line or "no event for" in line]
    print(f"=== {name} run {run} FAILED after {took[-1]:.1f}s, {len(lines)} lines")
    for line in panics:
        print("    " + line[:700])
    if len(failed) <= 3:
        for line in kept[:140]:
            print("  | " + line[:260])
    sys.stdout.flush()
if runs and stats:
    # What a run that passes looks like, to compare.
    passed = next(run for run in range(1, runs + 1) if run not in failed)
    with open(f"logs/{name}-{passed}.log") as log:
        kept = [line for line in log.read().splitlines() if re.match(r"playback audio|kerf playback", line)]
    print(f"=== {name} run {passed} passed; its first audio lines")
    for line in kept[:70]:
        print("  | " + line[:260])
mean = sum(took) / max(len(took), 1)
summary = (f"RESULT {name}: {len(failed)} of {runs} failed (runs {failed}), {mean:.1f}s a run, args {extra}, "
           f"stats {stats}; the device was set aside in {aside} runs and taken back in {back}")
with open(f"logs/{name}.result", "w") as result:
    result.write(summary + "\n")
print(summary)
