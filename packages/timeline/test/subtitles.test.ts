import { describe, it, expect } from "vitest";
import { TimelineSchema, applyEdit, type Timeline } from "../src/index.js";

const base: Timeline = TimelineSchema.parse({
  version: 1,
  fps: 30,
  sources: [{ id: "a", path: "/tmp/a.mp4" }],
  tracks: [{ id: "t1", clips: [{ id: "c1", src: "a", in: 0, out: 2 }] }],
  overlays: [],
});

describe("subtitles overlay schema", () => {
  it("parses a subtitles overlay with style", () => {
    const tl = TimelineSchema.parse({
      ...base,
      overlays: [
        {
          id: "subs1",
          type: "subtitles",
          path: "/tmp/subs.srt",
          style: {
            fontSize: 24,
            fontColor: "#ffffff",
            outlineColor: "#000000",
            position: "bottom",
            marginV: 40,
          },
        },
      ],
    });
    expect(tl.overlays[0]).toMatchObject({ type: "subtitles", path: "/tmp/subs.srt" });
  });

  it("rejects a subtitles overlay without path or id", () => {
    expect(() =>
      TimelineSchema.parse({ ...base, overlays: [{ type: "subtitles", path: "/x.srt" }] }),
    ).toThrow();
    expect(() =>
      TimelineSchema.parse({ ...base, overlays: [{ id: "s", type: "subtitles" }] }),
    ).toThrow();
  });
});

describe("add_subtitles / remove_overlay ops", () => {
  it("adds a subtitles overlay and removes it (round-trip)", () => {
    const added = applyEdit(base, { op: "add_subtitles", path: "/tmp/subs.srt" });
    expect(added.overlays).toHaveLength(1);
    const ov = added.overlays[0] as { id: string; type: string; path: string };
    expect(ov.type).toBe("subtitles");
    expect(ov.path).toBe("/tmp/subs.srt");
    expect(ov.id).toBeTruthy();

    const removed = applyEdit(added, { op: "remove_overlay", id: ov.id });
    expect(removed.overlays).toHaveLength(0);
  });

  it("keeps style through add/remove and does not mutate the input", () => {
    const added = applyEdit(base, {
      op: "add_subtitles",
      path: "/tmp/s.srt",
      style: { fontSize: 20, position: "top" },
    });
    expect((added.overlays[0] as { style?: unknown }).style).toEqual({
      fontSize: 20,
      position: "top",
    });
    expect(base.overlays).toHaveLength(0);
  });

  it("throws when removing an unknown overlay id", () => {
    expect(() => applyEdit(base, { op: "remove_overlay", id: "nope" })).toThrow(/nope/);
  });
});
