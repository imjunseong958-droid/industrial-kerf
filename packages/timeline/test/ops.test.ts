import { describe, it, expect } from "vitest";
import { applyEdit } from "../src/index.js";
import type { Timeline, EditOp } from "../src/index.js";

function mkTimeline(clips: { src?: string; in: number; out: number }[] = []): Timeline {
  return {
    version: 1,
    fps: 30,
    sources: [
      { id: "a", path: "/tmp/a.mp4" },
      { id: "b", path: "/tmp/b.mp4" },
    ],
    tracks: [
      {
        id: "v0",
        clips: clips.map((c, i) => ({
          id: `c${i + 1}`,
          src: c.src ?? "a",
          in: c.in,
          out: c.out,
        })),
      },
      { id: "v1", clips: [] },
    ],
    overlays: [],
  };
}

describe("trim", () => {
  it("adjusts in and out", () => {
    const t = applyEdit(mkTimeline([{ in: 0, out: 10 }]), {
      op: "trim",
      clipId: "c1",
      newIn: 2,
      newOut: 8,
    });
    expect(t.tracks[0]!.clips[0]).toMatchObject({ in: 2, out: 8 });
  });

  it("adjusts only one bound when the other is omitted", () => {
    const t = applyEdit(mkTimeline([{ in: 0, out: 10 }]), {
      op: "trim",
      clipId: "c1",
      newOut: 5,
    });
    expect(t.tracks[0]!.clips[0]).toEqual({ id: "c1", src: "a", in: 0, out: 5 });
  });

  it("does not mutate the original timeline", () => {
    const t0 = mkTimeline([{ in: 0, out: 10 }]);
    const t = applyEdit(t0, { op: "trim", clipId: "c1", newIn: 3 });
    expect(t0.tracks[0]!.clips[0]!.in).toBe(0);
    expect(t.tracks[0]!.clips[0]!.in).toBe(3);
  });

  it("throws for unknown clipId", () => {
    expect(() =>
      applyEdit(mkTimeline([{ in: 0, out: 10 }]), { op: "trim", clipId: "nope", newIn: 1 })
    ).toThrow(/not found/i);
  });

  it("throws when newIn >= newOut", () => {
    expect(() =>
      applyEdit(mkTimeline([{ in: 0, out: 10 }]), {
        op: "trim",
        clipId: "c1",
        newIn: 8,
        newOut: 8,
      })
    ).toThrow();
  });
});

describe("split", () => {
  it("cuts one clip into two at an absolute source time within the clip", () => {
    const t = applyEdit(mkTimeline([{ in: 0, out: 4 }, { in: 10, out: 20 }]), {
      op: "split",
      clipId: "c1",
      at: 2.5,
    });
    const clips = t.tracks[0]!.clips;
    expect(clips).toHaveLength(3);
    expect(clips[0]).toMatchObject({ in: 0, out: 2.5 });
    expect(clips[1]).toMatchObject({ in: 2.5, out: 4 });
    // untouched later clip
    expect(clips[2]).toMatchObject({ in: 10, out: 20 });
  });

  it("gives both halves distinct ids derived from the original", () => {
    const t = applyEdit(mkTimeline([{ in: 0, out: 4 }]), {
      op: "split",
      clipId: "c1",
      at: 2,
    });
    const ids = t.tracks[0]!.clips.map((c) => c.id);
    expect(new Set(ids).size).toBe(2);
    expect(ids).toContain("c1");
  });

  it("throws when at is outside the clip bounds", () => {
    expect(() =>
      applyEdit(mkTimeline([{ in: 0, out: 4 }]), { op: "split", clipId: "c1", at: 4 })
    ).toThrow();
    expect(() =>
      applyEdit(mkTimeline([{ in: 0, out: 4 }]), { op: "split", clipId: "c1", at: -1 })
    ).toThrow();
  });
});

describe("remove_ranges", () => {
  it("removes a range inside a single clip", () => {
    // track timeline: c1 covers 0..4
    const t = applyEdit(mkTimeline([{ in: 0, out: 4 }, { in: 10, out: 14 }]), {
      op: "remove_ranges",
      ranges: [{ start: 1, end: 2 }],
    });
    expect(t.tracks[0]!.clips.map((c) => [c.in, c.out])).toEqual([
      [0, 1],
      [2, 4],
      [10, 14], // source times unchanged; its timeline position shifted left by 1
    ]);
  });

  it("removes a range spanning multiple clips", () => {
    // c1: 0..4, c2: 4..8
    const t = applyEdit(mkTimeline([{ in: 0, out: 4 }, { in: 100, out: 104 }]), {
      op: "remove_ranges",
      ranges: [{ start: 2, end: 6 }],
    });
    expect(t.tracks[0]!.clips.map((c) => [c.in, c.out])).toEqual([
      [0, 2],   // head of c1
      [102, 104], // tail of c2
    ]);
  });

  it("deletes clips fully covered by a range", () => {
    const t = applyEdit(mkTimeline([{ in: 0, out: 2 }, { in: 50, out: 52 }, { in: 60, out: 64 }]), {
      op: "remove_ranges",
      ranges: [{ start: 1, end: 3 }],
    });
    // c1 (track 0-2) loses its tail; c2 (track 2-4) loses its first half; c3 kept
    expect(t.tracks[0]!.clips.map((c) => [c.in, c.out])).toEqual([
      [0, 1],
      [51, 52],
      [60, 64],
    ]);
  });

  it("handles a range beyond the end of the timeline (empty overlap)", () => {
    const t = applyEdit(mkTimeline([{ in: 0, out: 4 }]), {
      op: "remove_ranges",
      ranges: [{ start: 10, end: 20 }],
    });
    expect(t.tracks[0]!.clips.map((c) => [c.in, c.out])).toEqual([[0, 4]]);
  });

  it("handles multiple non-overlapping ranges", () => {
    // total duration 12
    const t = applyEdit(
      mkTimeline([{ in: 0, out: 6 }, { in: 100, out: 106 }]),
      {
        op: "remove_ranges",
        ranges: [
          { start: 1, end: 2 },
          { start: 4, end: 5 },
        ],
      }
    );
    expect(t.tracks[0]!.clips.map((c) => [c.in, c.out])).toEqual([
      [0, 1],
      [2, 4],
      [5, 6],
      [100, 106], // source times unchanged; only its timeline position shifts left
    ]);
  });

  it("handles overlapping/merged ranges gracefully", () => {
    const t = applyEdit(mkTimeline([{ in: 0, out: 10 }]), {
      op: "remove_ranges",
      ranges: [
        { start: 1, end: 3 },
        { start: 2, end: 5 },
      ],
    });
    expect(t.tracks[0]!.clips.map((c) => [c.in, c.out])).toEqual([
      [0, 1],
      [5, 10],
    ]);
  });

  it("can empty the first track entirely", () => {
    const t = applyEdit(mkTimeline([{ in: 0, out: 4 }]), {
      op: "remove_ranges",
      ranges: [{ start: 0, end: 4 }],
    });
    expect(t.tracks[0]!.clips).toHaveLength(0);
  });

  it("leaves other tracks untouched", () => {
    const t0 = mkTimeline([{ in: 0, out: 4 }]);
    t0.tracks[1]!.clips.push({ id: "x", src: "b", in: 7, out: 9 });
    const t = applyEdit(t0, {
      op: "remove_ranges",
      ranges: [{ start: 0, end: 2 }],
    });
    expect(t.tracks[1]!.clips).toEqual([{ id: "x", src: "b", in: 7, out: 9 }]);
  });

  it("throws on invalid ranges", () => {
    expect(() =>
      applyEdit(mkTimeline([{ in: 0, out: 4 }]), {
        op: "remove_ranges",
        ranges: [{ start: 3, end: 1 }],
      })
    ).toThrow();
  });
});

describe("set_speed / set_volume", () => {
  it("sets speed on a clip", () => {
    const t = applyEdit(mkTimeline([{ in: 0, out: 4 }]), {
      op: "set_speed",
      clipId: "c1",
      speed: 2,
    });
    expect(t.tracks[0]!.clips[0]!.speed).toBe(2);
  });

  it("sets volume on a clip", () => {
    const t = applyEdit(mkTimeline([{ in: 0, out: 4 }]), {
      op: "set_volume",
      clipId: "c1",
      volume: 0.25,
    });
    expect(t.tracks[0]!.clips[0]!.volume).toBe(0.25);
  });

  it("throws for unknown clip", () => {
    expect(() =>
      applyEdit(mkTimeline([]), { op: "set_speed", clipId: "z", speed: 1 })
    ).toThrow(/not found/i);
  });
});

describe("applyEdit dispatcher", () => {
  it("rejects unknown ops at runtime", () => {
    expect(() =>
      applyEdit(mkTimeline(), { op: "wat" } as unknown as EditOp)
    ).toThrow();
  });

  it("returns a structurally valid timeline after each op", () => {
    let t = mkTimeline([{ in: 0, out: 4 }, { in: 10, out: 14 }]);
    t = applyEdit(t, { op: "split", clipId: "c1", at: 2 });
    t = applyEdit(t, { op: "trim", clipId: "c1", newOut: 1 });
    t = applyEdit(t, { op: "remove_ranges", ranges: [{ start: 0.5, end: 1.5 }] });
    t = applyEdit(t, { op: "set_speed", clipId: "c1", speed: 1.5 });
    t = applyEdit(t, { op: "set_volume", clipId: "c2", volume: 0 });
    expect(t.version).toBe(1);
    for (const c of t.tracks[0]!.clips) expect(c.out).toBeGreaterThan(c.in);
  });
});
