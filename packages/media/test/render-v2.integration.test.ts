import { describe, expect, it } from "vitest";
import { execFileSync } from "node:child_process";
import { mkdtemp, rm, stat } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { ProjectSchema } from "@cut/timeline";
import { renderV2 } from "../src/render-v2.js";

let ffmpegOnPath = false;
try {
  execFileSync("ffmpeg", ["-version"], { stdio: "ignore" });
  ffmpegOnPath = true;
} catch {
  /* integration is skipped when ffmpeg is unavailable */
}

describe.skipIf(!ffmpegOnPath)("v2 render integration", () => {
  it("renders positioned video/audio tracks with a gap", { timeout: 120_000 }, async () => {
    const dir = await mkdtemp(path.join(os.tmpdir(), "cut-media-v2-"));
    try {
      const source = path.join(dir, "source.mp4");
      execFileSync(
        "ffmpeg",
        [
          "-y",
          "-f",
          "lavfi",
          "-i",
          "testsrc=duration=2:size=160x120:rate=24",
          "-f",
          "lavfi",
          "-i",
          "sine=frequency=440:duration=2",
          "-shortest",
          source,
        ],
        { stdio: "ignore" },
      );
      const project = ProjectSchema.parse({
        version: 2,
        fps: 24,
        width: 160,
        height: 120,
        duration: 2,
        sources: [{ id: "s", path: source }],
        tracks: [
          {
            id: "video",
            type: "video",
            clips: [{ id: "v", sourceId: "s", in: 0, out: 1, timelineStart: 0 }],
            gaps: [{ timelineStart: 1, duration: 1 }],
          },
          {
            id: "audio",
            type: "audio",
            clips: [{ id: "a", sourceId: "s", in: 0, out: 1, timelineStart: 1 }],
          },
        ],
      });
      const output = path.join(dir, "out.mp4");
      await renderV2(project, { output, draft: true });
      expect((await stat(output)).size).toBeGreaterThan(1000);
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });

  it("probes source duration when an interval and clip duration are omitted", { timeout: 120_000 }, async () => {
    const dir = await mkdtemp(path.join(os.tmpdir(), "cut-media-v2-duration-"));
    try {
      const source = path.join(dir, "source.mp4");
      execFileSync("ffmpeg", [
        "-y",
        "-f",
        "lavfi",
        "-i",
        "testsrc=duration=1:size=160x120:rate=24",
        "-an",
        "-c:v",
        "libx264",
        "-pix_fmt",
        "yuv420p",
        source,
      ], { stdio: "ignore" });
      const project = ProjectSchema.parse({
        version: 2,
        fps: 24,
        width: 160,
        height: 120,
        sources: [{ id: "s", path: source, width: 160, height: 120, hasAudio: false }],
        tracks: [{ id: "video", type: "video", clips: [{ id: "v", sourceId: "s", timelineStart: 0 }] }],
      });
      const output = path.join(dir, "out.mp4");
      await renderV2(project, { output, draft: true });
      expect((await stat(output)).size).toBeGreaterThan(1000);
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });
});
