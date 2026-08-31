import { describe, expect, it } from "vitest";
import {
  applyEdit,
  applyEdits,
  CropRegionSchema,
  EditOpSchema,
  TimelineSchema,
  type EditOp,
  type Timeline,
} from "../src/index.js";

function timeline(overrides: Partial<Timeline> = {}): Timeline {
  return {
    version: 1,
    fps: 30,
    sources: [{ id: "src", path: "/tmp/video.mp4", duration: 20 }],
    tracks: [
      {
        id: "v0",
        clips: [
          { id: "c1", src: "src", in: 0, out: 4 },
          { id: "c2", src: "src", in: 10, out: 14 },
        ],
      },
      { id: "v1", clips: [] },
    ],
    overlays: [],
    ...overrides,
  };
}

describe("timeline primitives", () => {
  it("supports source metadata, clips, crops, and inline captions", () => {
    const result = applyEdits(timeline(), [
      {
        op: "add_source",
        source: {
          id: "src2",
          path: "/tmp/second.mp4",
          duration: 8,
          width: 1920,
          height: 1080,
          hasAudio: true,
        },
      },
      {
        op: "add_clip",
        trackId: "v1",
        clip: {
          src: "src2",
          in: 1,
          out: 4,
          crop: { x: 0.1, y: 0.2, width: 0.5, height: 0.6 },
        },
      },
      { op: "set_crop", clipId: "c1", crop: { x: 0, y: 0, width: 1, height: 1 } },
      {
        op: "add_captions",
        cues: [{ start: 0, end: 1.5, text: "hello" }],
        style: { position: "bottom", fontSize: 24 },
      },
    ]);

    expect(result.sources[1]).toMatchObject({ width: 1920, height: 1080, hasAudio: true });
    expect(result.tracks[1]!.clips[0]).toMatchObject({ src: "src2", crop: { x: 0.1 } });
    expect(result.tracks[0]!.clips[0]!.crop).toEqual({ x: 0, y: 0, width: 1, height: 1 });
    expect(result.overlays[0]).toMatchObject({ type: "captions", cues: [{ text: "hello" }] });
    expect(new Set(result.tracks.flatMap((track) => track.clips.map((clip) => clip.id))).size).toBe(3);
  });

  it("moves and removes clips with validated destination/indexes", () => {
    const moved = applyEdit(timeline(), { op: "move_clip", clipId: "c1", toIndex: 1 });
    expect(moved.tracks[0]!.clips.map((clip) => clip.id)).toEqual(["c2", "c1"]);

    const crossTrack = applyEdit(timeline(), {
      op: "move_clip",
      clipId: "c1",
      trackId: "v1",
      toIndex: 0,
    });
    expect(crossTrack.tracks[0]!.clips.map((clip) => clip.id)).toEqual(["c2"]);
    expect(crossTrack.tracks[1]!.clips.map((clip) => clip.id)).toEqual(["c1"]);

    const removed = applyEdit(crossTrack, { op: "remove_clip", clipId: "c1" });
    expect(removed.tracks[1]!.clips).toEqual([]);
    expect(() => applyEdit(timeline(), { op: "add_clip", trackId: "nope", clip: { src: "src", in: 0, out: 1 } })).toThrow(/track/i);
    expect(() => applyEdit(timeline(), { op: "add_clip", index: 4, clip: { src: "src", in: 0, out: 1 } })).toThrow(/index/i);
  });

  it("maps remove_ranges through clip speed and keeps generated ids unique", () => {
    const result = applyEdit(
      timeline({
        tracks: [
          {
            id: "v0",
            clips: [{ id: "c1", src: "src", in: 0, out: 10, speed: 2 }],
          },
        ],
      }),
      { op: "remove_ranges", ranges: [{ start: 1, end: 2 }] },
    );
    expect(result.tracks[0]!.clips.map((clip) => [clip.in, clip.out])).toEqual([
      [0, 2],
      [4, 10],
    ]);
    expect(result.tracks[0]!.clips[0]!.speed).toBe(2);
    expect(new Set(result.tracks[0]!.clips.map((clip) => clip.id)).size).toBe(2);

    const split = applyEdits(result, [
      { op: "split", clipId: "c1", at: 1 },
      { op: "split", clipId: "c1", at: 0.5 },
    ]);
    expect(new Set(split.tracks[0]!.clips.map((clip) => clip.id)).size).toBe(
      split.tracks[0]!.clips.length,
    );
  });

  it("is atomic when the final timeline is invalid", () => {
    const original = timeline();
    const snapshot = structuredClone(original);
    expect(() =>
      applyEdits(original, [
        { op: "add_clip", clip: { src: "src", in: 0, out: 3, id: "new" } },
        { op: "trim", clipId: "new", newOut: 100 },
      ]),
    ).toThrow(/duration/i);
    expect(original).toEqual(snapshot);
  });

  it("rejects zero-sized crops", () => {
    expect(
      CropRegionSchema.safeParse({ x: 0, y: 0, width: 0, height: 1 }).success,
    ).toBe(false);
    expect(() =>
      applyEdit(timeline(), {
        op: "set_crop",
        clipId: "c1",
        crop: { x: 0, y: 0, width: 1, height: 0 },
      }),
    ).toThrow(/crop/i);
  });

  it("parses operations before dispatch and preserves the input on malformed ops", () => {
    const original = timeline();
    const snapshot = structuredClone(original);
    const malformed = {
      op: "add_clip",
      clip: { src: "src", in: 0, out: 1, speed: "fast" },
    } as unknown as EditOp;
    expect(() => applyEdits(original, [malformed])).toThrow();
    expect(original).toEqual(snapshot);
  });
});

describe("strong timeline schema", () => {
  it("rejects duplicate IDs, unknown sources, bad ranges, and crop bounds", () => {
    expect(() =>
      TimelineSchema.parse(
        timeline({
          tracks: [
            {
              id: "v0",
              clips: [
                { id: "same", src: "src", in: 0, out: 1 },
                { id: "same", src: "missing", in: 1, out: 2 },
              ],
            },
          ],
        }),
      ),
    ).toThrow();
    expect(() =>
      TimelineSchema.parse(
        timeline({
          tracks: [
            {
              id: "v0",
              clips: [{ id: "c1", src: "src", in: 0, out: 1, crop: { x: 0.8, y: 0, width: 0.5, height: 1 } }],
            },
          ],
        }),
      ),
    ).toThrow();
  });

  it("validates all new EditOp variants", () => {
    for (const op of [
      { op: "add_source", source: { id: "x", path: "/x" } },
      { op: "add_clip", clip: { src: "x", in: 0, out: 1 } },
      { op: "remove_clip", clipId: "c1" },
      { op: "move_clip", clipId: "c1", toIndex: 0 },
      { op: "set_crop", clipId: "c1", crop: null },
      { op: "add_captions", cues: [{ start: 0, end: 1, text: "x" }] },
    ]) {
      expect(EditOpSchema.safeParse(op).success).toBe(true);
    }
  });
});
