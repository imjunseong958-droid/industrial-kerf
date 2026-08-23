import { describe, it, expect } from "vitest";
import { execFileSync } from "node:child_process";
import { mkdtemp, stat, rm } from "node:fs/promises";
import path from "node:path";
import os from "node:os";
import type { Timeline } from "@cut/timeline";
import { render } from "../src/render.js";

let ffmpegOnPath = false;
try {
  execFileSync("ffmpeg", ["-version"], { stdio: "ignore" });
  ffmpegOnPath = true;
} catch {
  /* not installed */
}

describe.skipIf(!ffmpegOnPath)("render integration", () => {
  it(
    "renders a tiny generated testsrc video end-to-end",
    { timeout: 120_000 },
    async () => {
      const dir = await mkdtemp(path.join(os.tmpdir(), "cut-media-"));
      const srcA = path.join(dir, "a.mp4");
      const srcB = path.join(dir, "b.mp4");
      // 2s testsrc clips with audio
      for (const [p, color] of [
        [srcA, "red"],
        [srcB, "blue"],
      ] as const) {
        execFileSync(
          "ffmpeg",
          [
            "-y",
            "-f",
            "lavfi",
            "-i",
            `testsrc=duration=6:size=320x240:rate=30`,
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=6",
            "-vf",
            `hue=h=${color === "red" ? 0 : 120}`,
            "-shortest",
            p,
          ],
          { stdio: "ignore" },
        );
      }
      const timeline: Timeline = {
        version: 1,
        fps: 30,
        sources: [
          { id: "a", path: srcA },
          { id: "b", path: srcB },
        ],
        tracks: [
          {
            id: "t1",
            clips: [
              { src: "a", in: 1, out: 3, id: "c1" },
              { src: "b", in: 2, out: 5, speed: 2, volume: 0.8, id: "c2" },
            ],
          },
        ],
        overlays: [],
      };
      const output = path.join(dir, "out.mp4");
      const ratios: number[] = [];
      await render(timeline, {
        output,
        draft: true,
        onProgress: (r) => ratios.push(r),
      });
      const s = await stat(output);
      expect(s.size).toBeGreaterThan(1000);
      expect(ratios.length).toBeGreaterThan(0);
      expect(ratios[ratios.length - 1]).toBeGreaterThan(0.9);
      await rm(dir, { recursive: true, force: true });
    },
  );
});
