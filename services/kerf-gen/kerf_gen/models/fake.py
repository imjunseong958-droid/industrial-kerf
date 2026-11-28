"""A model for tests: it answers with its guide, at its own size and rate, else with a test
picture, written by FFmpeg. It runs anywhere FFmpeg does, so a test speaks to a real server
without a GPU.

`more` can say `hold`, seconds it waits before it starts, so that it can be asked to stop while
it runs.
"""
import os
import subprocess
import time

from . import Stopped

NAME = "fake"
WIDTH, HEIGHT = 160, 90
RATE = 12
SECONDS = 2

SPEC = {
    "about": "a model for tests: it answers with its guide, or a test picture",
    "size": [WIDTH, HEIGHT],
    "rate": RATE,
    "seconds": SECONDS,
    "reads": ["guide", "ref"],
    "more": {"hold": 0},
}

STILLS = (".png", ".jpg", ".jpeg", ".webp")


def ffmpeg():
    return os.environ.get("KERF_GEN_FFMPEG", "ffmpeg")


def generate(request, out, report, stopped):
    """Make what `request` asks, every file in it a path here, at `out`. `report(progress, line)`
    says how far it is, and `stopped()` whether it has been asked to stop."""
    hold = float(request.get("more", {}).get("hold", 0) or 0)
    ends = time.monotonic() + hold
    while time.monotonic() < ends:
        if stopped():
            raise Stopped()
        report(0.0, "holding")
        time.sleep(0.05)
    if stopped():
        raise Stopped()
    seconds = min(float(request.get("seconds") or SECONDS), SECONDS)
    guide = request.get("guide")
    still = os.path.splitext(out)[1].lower() in STILLS
    report(0.5, "answering with its guide" if guide else "drawing a test picture")
    picture = f"scale={WIDTH}:{HEIGHT}:force_original_aspect_ratio=increase,crop={WIDTH}:{HEIGHT},fps={RATE}"
    command = [ffmpeg(), "-hide_banner", "-loglevel", "error", "-y"]
    if guide:
        command += ["-i", guide]
    else:
        command += ["-f", "lavfi", "-i", f"testsrc2=size={WIDTH}x{HEIGHT}:rate={RATE}"]
    command += ["-vf", picture, "-an"]
    if still:
        command += ["-frames:v", "1"]
    else:
        command += ["-t", f"{seconds}", "-c:v", "libx264", "-pix_fmt", "yuv420p"]
    command.append(out)
    made = subprocess.run(command, capture_output=True, text=True)
    if made.returncode != 0:
        raise RuntimeError(f"ffmpeg failed: {made.stderr.strip()}")
    report(1.0, "made")
