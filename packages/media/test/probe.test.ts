import { spawnSync } from "node:child_process";
import { mkdtemp, readdir } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { beforeAll, describe, expect, it } from "vitest";
import {
  detectScenes,
  extractFrames,
  info,
  parseSceneTimes,
  parseSilencedetect,
  silenceDetect,
  type SilenceRange,
} from "../src/index.js";

const FFMPEG = spawnSync("ffmpeg", ["-version"], { encoding: "utf8" });
const hasFfmpeg = FFMPEG.status === 0;

describe("parseSilencedetect", () => {
  it("pairs start/end lines into ranges", () => {
    const stderr = [
      "[silencedetect @ 0x0] silence_start: 1.2",
      "[silencedetect @ 0x0] silence_end: 2.5 | silence_duration: 1.3",
      "[silencedetect @ 0x0] silence_start: 4",
      "[silencedetect @ 0x0] silence_end: 4.75 | silence_duration: 0.75",
    ].join("\n");
    expect(parseSilencedetect(stderr)).toEqual([
      { start: 1.2, end: 2.5 },
      { start: 4, end: 4.75 },
    ]);
  });

  it("handles trailing silence with no end (EOF)", () => {
    const stderr =
      "[silencedetect] silence_start: 3.1\n[silencedetect] silence_end: 3.9\n[silencedetect] silence_start: 8";
    expect(parseSilencedetect(stderr)).toEqual([
      { start: 3.1, end: 3.9 },
      { start: 8 },
    ] satisfies SilenceRange[]);
  });

  it("returns empty for no detections", () => {
    expect(parseSilencedetect("nothing here")).toEqual([]);
  });
});

describe("parseSceneTimes", () => {
  it("parses pts_time + scene_score pairs into consecutive ranges", () => {
    const stderr = [
      "frame:0    pts:0      pts_time:0.5",
      "lavfi.scene_score=0.721300",
      "frame:1    pts:48     pts_time:2.0",
      "lavfi.scene_score=0.512000",
    ].join("\n");
    expect(parseSceneTimes(stderr)).toEqual([
      { start: 0.5, end: 2 },
      { start: 2 },
    ]);
  });

  it("ignores scores without a preceding time and returns empty when none", () => {
    expect(parseSceneTimes("lavfi.scene_score=0.99\n")).toEqual([]);
  });
});

describe.skipIf(!hasFfmpeg)("integration (ffmpeg on PATH)", () => {
  let videoPath = "";

  /**
   * Build a 6s test video: testsrc video + audio that is silent in
   * [1s, 2s) and [3.5s, 5s), tone elsewhere.
   */
  beforeAll(async () => {
    const dir = await mkdtemp(path.join(tmpdir(), "cut-probe-"));
    videoPath = path.join(dir, "test.mp4");
    const r = spawnSync(
      "ffmpeg",
      [
        "-y",
        "-loglevel",
        "error",
        "-f",
        "lavfi",
        "-i",
        "testsrc=duration=6:size=320x240:rate=24",
        "-f",
        "lavfi",
        "-i",
        "aevalsrc=0.3*sin(880*2*PI*t):d=1:s=44100", // tone 0..1s
        "-f",
        "lavfi",
        "-i",
        "anullsrc=r=44100:cl=mono:d=1", // silence 1..2s
        "-f",
        "lavfi",
        "-i",
        "aevalsrc=0.3*sin(880*2*PI*t):d=1.5:s=44100", // tone 2..3.5s
        "-f",
        "lavfi",
        "-i",
        "anullsrc=r=44100:cl=mono:d=1.5", // silence 3.5..5s
        "-f",
        "lavfi",
        "-i",
        "aevalsrc=0.3*sin(880*2*PI*t):d=1:s=44100", // tone 5..6s
        "-filter_complex",
        "[1:a][2:a][3:a][4:a][5:a]concat=n=5:v=0:a=1[aout]",
        "-map",
        "0:v",
        "-map",
        "[aout]",
        "-c:v",
        "libx264",
        "-pix_fmt",
        "yuv420p",
        "-c:a",
        "aac",
        videoPath,
      ],
      { encoding: "utf8" },
    );
    if (r.status !== 0) throw new Error(`ffmpeg fixture failed: ${r.stderr}`);
  }, 30_000);

  it("info() returns normalized metadata", async () => {
    const i = await info(videoPath);
    expect(i.duration).toBeCloseTo(6, 0);
    expect(i.width).toBe(320);
    expect(i.height).toBe(240);
    expect(i.fps).toBeCloseTo(24, 0);
    expect(i.hasAudio).toBe(true);
    expect(i.videoCodec).toBeTruthy();
    expect(i.audioCodec).toBeTruthy();
  });

  it("silenceDetect() finds the two silent segments", async () => {
    const ranges = await silenceDetect(videoPath, {
      thresholdDb: -35,
      minDurationSec: 0.4,
    });
    expect(ranges.length).toBeGreaterThanOrEqual(2);
    // A range should cover the middle of each engineered silent window.
    const covers = (t: number) =>
      ranges.some((r) => r.start < t && (r.end ?? Infinity) > t);
    expect(covers(1.5)).toBe(true);
    expect(covers(4.25)).toBe(true);
  }, 30_000);

  it("detectScenes() parses without error (static testsrc may yield none)", async () => {
    const scenes = await detectScenes(videoPath, { threshold: 0.9 });
    expect(Array.isArray(scenes)).toBe(true);
    for (const s of scenes) expect(s.start).toBeGreaterThanOrEqual(0);
  }, 60_000);

  it("extractFrames() writes one PNG per timestamp", async () => {
    const outDir = path.join(path.dirname(videoPath), "frames");
    const frames = await extractFrames(videoPath, [0.5, 3], outDir);
    expect(frames).toHaveLength(2);
    for (const f of frames) {
      const files = await readdir(outDir);
      expect(files.some((n) => n === path.basename(f.path))).toBe(true);
    }
  }, 30_000);
});
