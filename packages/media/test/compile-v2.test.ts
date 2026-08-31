import { describe, expect, it } from "vitest";
import { ProjectSchema, type Project } from "@cut/timeline";
import {
  compileFilterGraphV2,
  timelineDurationV2,
  UnsupportedV2FeatureError,
} from "../src/compile-v2.js";

function project(overrides: Record<string, unknown> = {}): Project {
  return ProjectSchema.parse({
    version: 2,
    fps: 30,
    width: 320,
    height: 180,
    sources: [
      { id: "a", path: "/tmp/a.mp4", width: 320, height: 180, hasAudio: true },
      { id: "b", path: "/tmp/b.mp4", width: 320, height: 180, hasAudio: true },
    ],
    tracks: [
      {
        id: "video-a",
        type: "video",
        clips: [{ id: "a1", sourceId: "a", in: 0, out: 2, timelineStart: 0 }],
        gaps: [{ timelineStart: 2, duration: 1 }],
      },
      {
        id: "video-b",
        type: "video",
        clips: [{ id: "b1", sourceId: "b", in: 0, out: 1, timelineStart: 1 }],
      },
      {
        id: "audio",
        type: "audio",
        clips: [{ id: "a2", sourceId: "a", in: 0, out: 1, timelineStart: 0, volume: 0.5 }],
      },
    ],
    ...overrides,
  });
}

describe("compileFilterGraphV2", () => {
  it("composites positioned video tracks and mixes audio tracks", () => {
    const compiled = compileFilterGraphV2(project());
    expect(compiled.duration).toBe(3);
    expect(compiled.filterComplex).toContain("overlay=");
    expect(compiled.filterComplex).toContain("adelay=0:all=1");
    expect(compiled.filterComplex).toContain("amix=inputs=2"); // silence bed + explicit audio clip
    expect(compiled.filterComplex).not.toContain("[1:a]"); // embedded video audio is suppressed by policy
    expect(compiled.filterComplex).toContain("d=3");
    // Track streams are composited into one canvas; v2 must not concatenate
    // independent tracks as v1 does.
    expect(compiled.filterComplex).not.toContain("concat=");
  });

  it("uses a silence stream when direct compilation lacks audio metadata", () => {
    const parsed = project({
      sources: [{ id: "video", path: "/tmp/video-only.mp4", width: 320, height: 180 }],
      tracks: [
        {
          id: "video",
          type: "video",
          clips: [{ id: "clip", sourceId: "video", in: 0, out: 1, timelineStart: 0 }],
        },
      ],
    });
    const filter = compileFilterGraphV2(parsed).filterComplex;
    expect(filter).not.toContain("[0:a]");
    expect(filter).toContain("aevalsrc=0:c=stereo");
  });

  it("emits opacity, normalized crop/transform, and linear keyframe expressions", () => {
    const parsed = project({
      tracks: [
        {
          id: "video",
          type: "video",
          clips: [
            {
              id: "c",
              sourceId: "a",
              in: 0,
              out: 2,
              timelineStart: 1,
              opacity: 0.75,
              crop: { x: 0.1, y: 0.2, width: 0.5, height: 0.6 },
              transform: { x: 0.2, y: -0.1, scale: 0.8, rotation: 4 },
              keyframes: [
                { property: "opacity", time: 0, value: 0 },
                { property: "opacity", time: 2, value: 1 },
                { property: "transform.x", time: 0, value: 0 },
                { property: "transform.x", time: 2, value: 0.2 },
              ],
            },
          ],
        },
      ],
    });
    const filter = compileFilterGraphV2(parsed).filterComplex;
    expect(filter).toContain("crop=w=");
    expect(filter).toContain("geq=r='r(X,Y)'");
    expect(filter).toContain("if(lt(T\\,2)");
    expect(filter).toContain("scale=w=");
    expect(filter).toContain("rotate=");
    expect(filter).toContain("main_w*(if(lt((t-1)\\,2)");
  });

  it("accounts for explicit placements and gaps in duration", () => {
    const parsed = project({
      duration: undefined,
      tracks: [
        {
          id: "video",
          type: "video",
          clips: [{ id: "c", sourceId: "a", in: 0, out: 1, timelineStart: 4 }],
          gaps: [{ start: 0, duration: 2 }],
        },
      ],
    });
    expect(timelineDurationV2(parsed)).toBe(5);
    expect(compileFilterGraphV2(parsed).duration).toBe(5);
  });

  it("includes project-level gaps in duration and rejects project effects/transitions", () => {
    const parsed = project({
      duration: 1,
      gaps: [{ timelineStart: 3, duration: 1 }],
    });
    const compiled = compileFilterGraphV2(parsed);
    expect(compiled.duration).toBe(4);
    expect(compiled.filterComplex).toContain("d=4");

    const effects = project({ effects: [{ type: "blur" }] });
    expect(() => compileFilterGraphV2(effects)).toThrow(/effect.*project/i);
    const transitions = project({ transitions: [{ type: "fade", duration: 0.5 }] });
    expect(() => compileFilterGraphV2(transitions)).toThrow(/project transition/i);
  });

  it("trims an explicitly shorter logical clip duration after speed", () => {
    const parsed = project({
      tracks: [
        {
          id: "video",
          type: "video",
          clips: [{ id: "c", sourceId: "a", in: 0, out: 2, duration: 0.5, timelineStart: 0 }],
        },
      ],
    });
    const filter = compileFilterGraphV2(parsed).filterComplex;
    expect(filter).toContain("trim=duration=0.5,setpts=PTS-STARTPTS");
  });

  it("rejects unsupported effects instead of dropping them", () => {
    const parsed = project({
      tracks: [
        {
          id: "video",
          type: "video",
          clips: [
            {
              id: "c",
              sourceId: "a",
              in: 0,
              out: 1,
              timelineStart: 0,
              effects: [{ type: "blur" }],
            },
          ],
        },
      ],
    });
    expect(() => compileFilterGraphV2(parsed)).toThrow(UnsupportedV2FeatureError);
    expect(() => compileFilterGraphV2(parsed)).toThrow(/effect.*blur/i);
  });

  it("treats an identity color grade as a no-op", () => {
    const parsed = project({
      tracks: [
        {
          id: "video",
          type: "video",
          clips: [
            {
              id: "c",
              sourceId: "a",
              in: 0,
              out: 1,
              timelineStart: 0,
              effects: [{ type: "color" }],
            },
          ],
        },
      ],
    });
    const filter = compileFilterGraphV2(parsed).filterComplex;
    expect(filter).not.toContain("exposure=");
    expect(filter).not.toContain("eq=");
    expect(filter).not.toContain("colorbalance=");
    expect(filter).not.toContain("vibrance=");
  });

  it("compiles color controls before opacity/composition", () => {
    const parsed = project({
      tracks: [
        {
          id: "video",
          type: "video",
          clips: [
            {
              id: "c",
              sourceId: "a",
              in: 0,
              out: 1,
              timelineStart: 0,
              opacity: 0.75,
              effects: [
                {
                  type: "color",
                  params: {
                    exposure: 1,
                    contrast: 1.2,
                    brightness: 0.1,
                    saturation: 1.3,
                    temperature: 320,
                    tint: 0.1,
                    gamma: 1.2,
                    highlights: 0.2,
                    shadows: -0.1,
                    vibrance: 0.4,
                    mix: 1,
                  },
                },
              ],
            },
          ],
        },
      ],
    });
    const filter = compileFilterGraphV2(parsed).filterComplex;
    expect(filter).toContain("exposure=exposure=1");
    expect(filter).toContain("eq=contrast=1.2:brightness=0.1:saturation=1.3:gamma=1.2");
    expect(filter).toContain("colorbalance=");
    expect(filter).toContain("vibrance=intensity=0.4");
    expect(filter.indexOf("vibrance="))
      .toBeLessThan(filter.indexOf("colorchannelmixer=aa=0.75"));
  });

  it("validates color ranges and rejects unsupported LUT paths", () => {
    expect(() =>
      project({
        tracks: [
          {
            id: "video",
            type: "video",
            clips: [
              {
                id: "c",
                sourceId: "a",
                in: 0,
                out: 1,
                timelineStart: 0,
                effects: [{ type: "color", params: { contrast: 9 } }],
              },
            ],
          },
        ],
      }),
    ).toThrow(/contrast/i);

    const withLut = project({
      tracks: [
        {
          id: "video",
          type: "video",
          clips: [
            {
              id: "c",
              sourceId: "a",
              in: 0,
              out: 1,
              timelineStart: 0,
              effects: [{ type: "color", params: { lut: "/tmp/look.cube" } }],
            },
          ],
        },
      ],
    });
    expect(() => compileFilterGraphV2(withLut)).toThrow(/color LUT.*not supported/i);
  });

  it("supports only validated overlapping fade/crossfade/dissolve transitions", () => {
    const parsed = project({
      tracks: [
        {
          id: "video",
          type: "video",
          clips: [
            {
              id: "a1",
              sourceId: "a",
              in: 0,
              out: 2,
              timelineStart: 0,
              transitions: [{ type: "crossfade", duration: 0.5 }],
            },
            { id: "b1", sourceId: "b", in: 0, out: 1.5, timelineStart: 1.5 },
          ],
        },
      ],
    });
    const filter = compileFilterGraphV2(parsed).filterComplex;
    expect(filter).toContain("xfade=transition=fade:duration=0.5:offset=1.5");

    const unsupported = project({
      tracks: [
        {
          id: "video",
          type: "video",
          clips: [
            {
              id: "a1",
              sourceId: "a",
              in: 0,
              out: 1,
              timelineStart: 0,
              transitions: [{ type: "wipeleft", duration: 0.25 }],
            },
            { id: "b1", sourceId: "b", in: 0, out: 1, timelineStart: 0.75 },
          ],
        },
      ],
    });
    expect(() => compileFilterGraphV2(unsupported)).toThrow(/transition/i);
  });

  it("normalizes differing transition source dimensions before xfade", () => {
    const parsed = project({
      width: 640,
      height: 360,
      sources: [
        { id: "a", path: "/tmp/a.mp4", width: 640, height: 360, hasAudio: true },
        { id: "b", path: "/tmp/b.mp4", width: 1280, height: 720, hasAudio: true },
      ],
      tracks: [
        {
          id: "video",
          type: "video",
          clips: [
            {
              id: "a1",
              sourceId: "a",
              in: 0,
              out: 2,
              timelineStart: 0,
              transitions: [{ type: "fade", duration: 0.5 }],
            },
            { id: "b1", sourceId: "b", in: 0, out: 1.5, timelineStart: 1.5 },
          ],
        },
      ],
    });
    const filter = compileFilterGraphV2(parsed).filterComplex;
    expect(filter.match(/scale=640:360:force_original_aspect_ratio=decrease/g)?.length).toBe(2);
    expect(filter).toContain("fps=30,format=yuv420p");
    expect(filter).toContain("xfade=transition=fade");
  });

  it("preserves migrated subtitle and inline caption overlays", () => {
    const parsed = project({
      overlays: [
        { id: "s", type: "subtitles", path: "/tmp/subs.srt" },
        {
          id: "c",
          type: "captions",
          cues: [{ start: 0.25, end: 0.75, text: "Hello" }],
        },
      ],
    });
    const filter = compileFilterGraphV2(parsed).filterComplex;
    expect(filter).toContain("subtitles=filename='/tmp/subs.srt'");
    expect(filter).toContain("drawtext=text=Hello");
    expect(filter).toContain("between(t,0.25,0.75)");
  });

  it("uses embedded video audio only when no explicit audio track exists", () => {
    const videoOnly = project({
      tracks: [
        {
          id: "video",
          type: "video",
          clips: [{ id: "v", sourceId: "a", in: 0, out: 1, timelineStart: 0 }],
        },
      ],
    });
    const embedded = compileFilterGraphV2(videoOnly).filterComplex;
    expect(embedded).toContain("[0:a]");

    const withExplicit = project({
      tracks: [
        {
          id: "video",
          type: "video",
          clips: [{ id: "v", sourceId: "a", in: 0, out: 1, timelineStart: 0 }],
        },
        {
          id: "audio",
          type: "audio",
          clips: [{ id: "a", sourceId: "a", in: 0, out: 1, timelineStart: 0 }],
        },
      ],
    });
    const explicit = compileFilterGraphV2(withExplicit).filterComplex;
    expect(explicit.match(/\[0:a\]/g)?.length).toBe(1);

    const optedIn = compileFilterGraphV2(withExplicit, { includeEmbeddedAudio: true }).filterComplex;
    expect(optedIn.match(/\[0:a\]/g)?.length).toBe(2);
  });
});
