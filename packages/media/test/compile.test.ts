import { describe, it, expect } from "vitest";
import {
  TimelineSchema,
  type Timeline,
} from "@cut/timeline";
import {
  atempoFilters,
  captionsFilter,
  compileFilterGraph,
  timelineDuration,
} from "../src/compile.js";
import {
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
    // no shell metacharacter wrapping, plain args
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

describe("compileFilterGraph reliability", () => {
  it("allocates independent labels when the same source is used repeatedly", () => {
    const timeline = {
      version: 1,
      fps: 30,
      sources: [{ id: "same", path: "/tmp/repeated.mp4", hasAudio: false }],
      tracks: [
        {
          id: "t1",
          clips: [
            { id: "one", src: "same", in: 0, out: 1 },
            { id: "two", src: "same", in: 1, out: 2 },
          ],
        },
      ],
      overlays: [],
    } as unknown as Timeline;
    const { filterComplex } = compileFilterGraph(timeline);
    expect(filterComplex).toMatch(/\[0:v\][^;]*\[clipv_0\]/);
    expect(filterComplex).toMatch(/\[0:v\][^;]*\[clipv_2\]/);
    expect(filterComplex).toContain("anullsrc=r=48000:cl=stereo");
    expect(filterComplex).toContain("concat=n=2:v=1:a=1");
  });

  it("skips empty tracks and rejects a timeline with no clips", () => {
    const timeline = {
      version: 1,
      fps: 30,
      sources: [{ id: "a", path: "/tmp/a.mp4" }],
      tracks: [
        { id: "empty", clips: [] },
        { id: "used", clips: [{ id: "c", src: "a", in: 0, out: 1 }] },
      ],
      overlays: [],
    } as unknown as Timeline;
    expect(() => compileFilterGraph(timeline)).not.toThrow();
    expect(() =>
      compileFilterGraph({ ...timeline, tracks: [{ id: "empty", clips: [] }] }),
    ).toThrow(/empty timeline/i);
  });

  it("uses known dimensions for the canvas and emits a safe crop", () => {
    const timeline = {
      version: 1,
      fps: 24,
      sources: [{ id: "a", path: "/tmp/a.mp4", width: 320, height: 240 }],
      tracks: [
        {
          id: "t",
          clips: [
            {
              id: "c",
              src: "a",
              in: 0,
              out: 1,
              crop: { x: 0.25, y: 0.1, width: 0.5, height: 0.75 },
            },
          ],
        },
      ],
      overlays: [],
    } as unknown as Timeline;
    const { filterComplex } = compileFilterGraph(timeline);
    expect(filterComplex).toContain("crop=w=");
    expect(filterComplex).toContain("scale=320:240:");
    expect(filterComplex).not.toContain("scale=1920:1080");
  });

  it("uses one largest complete source aspect for portrait/landscape mixes", () => {
    const timeline = {
      version: 1,
      fps: 30,
      sources: [
        { id: "landscape", path: "/tmp/landscape.mp4", width: 1280, height: 720 },
        { id: "portrait", path: "/tmp/portrait.mp4", width: 1080, height: 1920 },
      ],
      tracks: [
        {
          id: "t",
          clips: [
            { id: "l", src: "landscape", in: 0, out: 1 },
            { id: "p", src: "portrait", in: 0, out: 1 },
          ],
        },
      ],
      overlays: [],
    } as unknown as Timeline;
    const { filterComplex } = compileFilterGraph(timeline);
    expect(filterComplex).toContain("scale=1080:1920:");
    expect(filterComplex).not.toContain("scale=1280:1920:");
    expect(filterComplex).not.toContain("scale=1920:1920:");
  });

  it("keeps atempo factors within ffmpeg bounds for arbitrary speeds", () => {
    for (const speed of [0.1, 0.25, 0.5, 2, 100, 10_000]) {
      const factors = atempoFilters(speed).map((f) => Number(f.slice("atempo=".length)));
      expect(factors.every((factor) => factor >= 0.5 && factor <= 100)).toBe(true);
      expect(factors.reduce((product, factor) => product * factor, 1)).toBeCloseTo(speed, 8);
    }
  });

  it("escapes timed caption text and maps its style/position", () => {
    const filter = captionsFilter(
      { start: 1, end: 2.5, text: "A:b,c; it's 100%" },
      { fontSize: 22, fontColor: "#ffffff", position: "top", marginV: 18 },
    );
    expect(filter).toContain("drawtext=text=");
    expect(filter).toContain("between(t,1,2.5)");
    expect(filter).toContain("fontsize=22");
    expect(filter).toContain("fontcolor=0xffffff");
    expect(filter).toContain("y='18'");
    expect(filter).toContain("A\\\\:b");
    expect(filter).toContain("100\\\\%");
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
