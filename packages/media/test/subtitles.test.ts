import { describe, it, expect } from "vitest";
import { execFileSync } from "node:child_process";
import { mkdtemp, stat, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import os from "node:os";
import type { Timeline } from "@cut/timeline";
import { escapeFilterPath, subtitlesFilter, compileFilterGraph } from "../src/compile.js";
import { render } from "../src/render.js";

const FFMPEG = "/opt/homebrew/bin/ffmpeg";

let ffmpegPresent = false;
try {
  execFileSync(FFMPEG, ["-version"], { stdio: "ignore" });
  ffmpegPresent = true;
} catch {
  /* not installed */
}

describe("subtitles burning in compileFilterGraph", () => {
  it("escapes colons, quotes and backslashes in paths", () => {
    expect(escapeFilterPath("/a/b:c'd\\e.srt")).toBe("/a/b\\:c\\'d\\\\e.srt");
  });

  it("maps style to force_style params", () => {
    const f = subtitlesFilter({
      id: "s1",
      type: "subtitles",
      path: "/tmp/subs.srt",
      style: {
        fontSize: 24,
        fontColor: "#ffffff",
        outlineColor: "#000000",
        position: "bottom",
        marginV: 40,
      },
    });
    expect(f).toContain("subtitles=filename='/tmp/subs.srt'");
    expect(f).toContain("FontSize=24");
    expect(f).toContain("PrimaryColour=&H00FFFFFF");
    expect(f).toContain("OutlineColour=&H00000000");
    expect(f).toContain("Alignment=2");
    expect(f).toContain("MarginV=40");
  });

  it("chains one subtitles filter per overlay after the concat output", () => {
    const tl: Timeline = {
      version: 1,
      fps: 30,
      sources: [{ id: "a", path: "/tmp/a.mp4" }],
      tracks: [{ id: "t1", clips: [{ id: "c1", src: "a", in: 0, out: 2 }] }],
      overlays: [
        { id: "s1", type: "subtitles", path: "/tmp/one:s.srt" },
        { id: "s2", type: "subtitles", path: "/tmp/two.srt" },
      ],
    };
    const { filterComplex, argv } = compileFilterGraph(tl);
    // escaped colon
    expect(filterComplex).toContain("/tmp/one\\:s.srt");
    expect(filterComplex.match(/subtitles=filename=/g)).toHaveLength(2);
    // final mapped label is the last subtitles output
    expect(argv[argv.indexOf("-map") + 1]).toContain("subs");
    expect(filterComplex.split(";").at(-1)).toMatch(/subtitles=/);
  });
});

describe.skipIf(!ffmpegPresent)("subtitles render integration", () => {
  it(
    "renders burned-in subtitles into a nonzero output",
    { timeout: 120_000 },
    async () => {
      const dir = await mkdtemp(path.join(os.tmpdir(), "cut-subs-"));
      const src = path.join(dir, "a.mp4");
      execFileSync(
        FFMPEG,
        [
          "-y",
          "-f", "lavfi",
          "-i", "testsrc=duration=3:size=320x240:rate=30",
          "-f", "lavfi",
          "-i", "sine=frequency=440:duration=3",
          "-shortest",
          src,
        ],
        { stdio: "ignore" },
      );
      const srtPath = path.join(dir, "subs.srt");
      await writeFile(
        srtPath,
        "1\n00:00:00,000 --> 00:00:03,000\nHello subtitles\n",
      );
      const tl: Timeline = {
        version: 1,
        fps: 30,
        sources: [{ id: "a", path: src }],
        tracks: [{ id: "t1", clips: [{ id: "c1", src: "a", in: 0, out: 2 }] }],
        overlays: [
          {
            id: "subs1",
            type: "subtitles",
            path: srtPath,
            style: { fontSize: 20, fontColor: "#ffffff", position: "bottom" },
          },
        ],
      };
      const output = path.join(dir, "out.mp4");
      await render(tl, { output });
      const s = await stat(output);
      expect(s.size).toBeGreaterThan(1000);
      await rm(dir, { recursive: true, force: true });
    },
  );
});
