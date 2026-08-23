import { describe, it, expect } from "vitest";
import {
  TimelineSchema,
  type Timeline,
} from "@cut/timeline";
import { compileFilterGraph, timelineDuration } from "../src/compile.js";
import {
  parseTimeSeconds,
  renderArgs,
} from "../src/render.js";

const twoClipTimeline: Timeline = TimelineSchema.parse({
  version: 1,
  fps: 30,
  sources: [{ id: "a", path: "/tmp/a.mp4" }, { id: "b", path: "/tmp/b.mp4" }],
  tracks: [
    {
      id: "t1",
      clips: [
        { src: "a", in: 0, out: 2, id: "c1" },
        { src: "b", in: 5, out: 8, speed: 2, volume: 0.5, id: "c2" },
      ],
    },
  ],
  overlays: [],
});

describe("compileFilterGraph", () => {
  const { command, filterComplex, argv } = compileFilterGraph(twoClipTimeline);

  it("contains trim filters for both clips", () => {
    expect(filterComplex).toContain("trim=start=0:end=2");
    expect(filterComplex).toContain("atrim=start=5:end=8");
  });

  it("contains a concat filter across the clips of the track", () => {
    expect(filterComplex).toContain("concat=n=2:v=1:a=1");
  });

  it("applies setpts and atempo for speed, volume for volume", () => {
    expect(filterComplex).toContain("setpts=0.5*PTS");
    expect(filterComplex).toContain("atempo=2");
    expect(filterComplex).toContain("volume=0.5");
  });

  it("produces an argv array starting with ffmpeg and array-safe args", () => {
    expect(Array.isArray(argv)).toBe(true);
    expect(argv[0]).toBe("ffmpeg");
    const fcIdx = argv.indexOf("-filter_complex");
    expect(typeof argv[fcIdx! + 1]).toBe("string");
    // no shell metacharacter wrapping — plain args
    expect(command).not.toContain("&&");
  });

  it("throws on unknown source id", () => {
    const bad = structuredClone(twoClipTimeline);
    bad.tracks[0]!.clips[0]!.src = "nope";
    expect(() => compileFilterGraph(bad)).toThrow(/Unknown source/);
  });

  it("computes duration accounting for speed", () => {
    expect(timelineDuration(twoClipTimeline)).toBe(3.5); // 2 + 3/2
  });
});

describe("parseTimeSeconds", () => {
  it("parses HH:MM:SS.xx and raw seconds", () => {
    expect(parseTimeSeconds("00:01:02.50")).toBeCloseTo(62.5);
    expect(parseTimeSeconds("12.34")).toBeCloseTo(12.34);
    expect(parseTimeSeconds("garbage")).toBeUndefined();
  });
});

describe("renderArgs", () => {
  it("draft mode uses 360p / ultrafast / crf35", () => {
    const args = renderArgs(twoClipTimeline, { draft: true, output: "out.mp4" });
    const fc = args[args.indexOf("-filter_complex") + 1]!;
    expect(fc).toContain("scale=-2:360");
    expect(fc).toContain("fps=15");
    expect(args).toContain("ultrafast");
    expect(args.at(-1)).toBe("out.mp4");
  });
});
