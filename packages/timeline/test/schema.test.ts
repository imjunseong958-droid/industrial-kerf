import { describe, it, expect } from "vitest";
import { TimelineSchema, EditOpSchema } from "../src/index.js";

describe("timeline", () => {
  it("parses a valid empty timeline", () => {
    const t = {
      version: 1,
      fps: 30,
      sources: [{ id: "a", path: "/tmp/a.mp4" }],
      tracks: [{ id: "v0", clips: [{ src: "a", in: 0, out: 5 }] }],
      overlays: [],
    };
    expect(TimelineSchema.parse(t)).toMatchObject({ fps: 30 });
  });

  it("rejects an invalid timeline", () => {
    expect(() => TimelineSchema.parse({ version: 2 })).toThrow();
  });

  it("accepts every EditOp variant and rejects unknown ops", () => {
    expect(EditOpSchema.safeParse({ op: "split", clipId: "c1", at: 3 }).success).toBe(true);
    expect(
      EditOpSchema.safeParse({ op: "remove_ranges", ranges: [{ start: 1, end: 2 }] }).success
    ).toBe(true);
    expect(EditOpSchema.safeParse({ op: "nope" }).success).toBe(false);
  });
});
