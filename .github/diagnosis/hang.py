#!/usr/bin/env python3
"""Run the kerf-mcp harness tests over and over on macOS; when a run does not end, say where it is.

usage: hang.py RUNS SECONDS

A run that takes longer than SECONDS is sampled (every thread's stack, by macOS's `sample`), its
ffmpeg children are listed with their arguments and sampled, and it is killed.
"""
import os, re, subprocess, sys, time, glob

runs, limit = int(sys.argv[1]), float(sys.argv[2])
os.makedirs("logs", exist_ok=True)
binary = sorted(glob.glob("target/debug/deps/harness-*"), key=os.path.getmtime)
binary = [b for b in binary if not b.endswith(".d") and os.access(b, os.X_OK)][-1]
print("test binary", binary, flush=True)
hung = 0
for run in range(1, runs + 1):
    log = open(f"logs/harness-{run}.log", "w")
    started = time.time()
    test = subprocess.Popen([binary], stdout=log, stderr=subprocess.STDOUT, cwd="crates/surfaces/kerf-mcp")
    while test.poll() is None and time.time() - started < limit:
        time.sleep(1)
    took = time.time() - started
    if test.poll() is not None:
        tail = open(f"logs/harness-{run}.log").read().splitlines()[-1:]
        print(f"run {run}: ended in {took:.0f}s, code {test.returncode}: {tail}", flush=True)
        continue
    hung += 1
    print(f"=== run {run} did not end in {limit:.0f}s", flush=True)
    for line in open(f"logs/harness-{run}.log").read().splitlines()[-8:]:
        print("  log| " + line[:200])
    ps = subprocess.run(["ps", "-axo", "pid,ppid,stat,etime,command"], capture_output=True, text=True).stdout
    children = [l for l in ps.splitlines() if re.search(r"ffmpeg|ffprobe|kerfd|harness-", l)]
    for line in children:
        print("  ps | " + line[:900], flush=True)
    out = f"logs/sample-{run}.txt"
    subprocess.run(["sample", str(test.pid), "3", "-file", out], capture_output=True, text=True)
    text = open(out).read() if os.path.exists(out) else ""
    # The call graph: keep our frames and what they wait in.
    shown = 0
    for line in text.splitlines():
        if re.search(r"Thread_|kerf_|harness::|ffmpeg|wait|lock|recv|read|park|cond|sem|Metal|wgpu|IOSurface|VT|avcodec", line) and shown < 420:
            print("  smp| " + line[:230])
            shown += 1
    for line in children:
        pid = line.split()[0]
        if "ffmpeg" in line and pid.isdigit():
            fout = f"logs/sample-{run}-ffmpeg-{pid}.txt"
            subprocess.run(["sample", pid, "2", "-file", fout], capture_output=True, text=True)
            ftext = open(fout).read() if os.path.exists(fout) else ""
            kept = [l for l in ftext.splitlines() if re.search(r"Thread_|main|read|write|wait|lock|VT|avcodec|av_|pipe|select|poll", l)][:60]
            print(f"  --- ffmpeg {pid}")
            for l in kept:
                print("  ffm| " + l[:200])
    subprocess.run(["pkill", "-9", "-f", "harness-"])
    subprocess.run(["pkill", "-9", "ffmpeg"])
    test.wait()
    sys.stdout.flush()
print(f"RESULT harness: {hung} of {runs} runs did not end in {limit:.0f}s")
