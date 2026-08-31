import { describe, expect, it } from "vitest";
import {
  ProjectCommandSchema,
  ProjectSchema,
  GapSchema,
  MarkerSchema,
  TimelineClipSchema,
  TrackV2Schema,
  TimelineV2Schema,
  emptyProject,
  applyProject,
  applyProjectCommand,
  applyProjectCommands,
  canRedo,
  canUndo,
  createProjectHistory,
  migrateV1ToV2,
  redoProject,
  undoProject,
  type Project,
} from "../src/index.js";

function project(): Project {
  return ProjectSchema.parse({
    version: 2,
    fps: 30,
    sources: [{ id: "src", path: "/tmp/video.mp4", duration: 10 }],
    tracks: [
      {
        id: "video",
        type: "video",
        clips: [{ id: "c1", sourceId: "src", in: 0, out: 4, timelineStart: 0 }],
      },
      { id: "titles", type: "title", clips: [] },
    ],
  });
}

describe("v2 project IR", () => {
  it("creates a valid empty project template", () => {
    const project = emptyProject();
    expect(project).toMatchObject({
      version: 2,
      name: "Untitled project",
      fps: 30,
      sources: [],
      tracks: [
        { id: "video-1", type: "video", clips: [] },
        { id: "audio-1", type: "audio", clips: [] },
      ],
      overlays: [],
      gaps: [],
      transitions: [],
      effects: [],
      markers: [],
    });
    expect(ProjectSchema.safeParse(project).success).toBe(true);
  });

  it("validates explicit placement and title/adjustment features", () => {
    const parsed = TimelineV2Schema.parse({
      version: 2,
      fps: 24,
      sources: [{ id: "s", path: "/tmp/s.mp4" }],
      tracks: [
        {
          id: "v",
          kind: "video",
          clips: [
            {
              id: "v1",
              src: "s",
              in: 0,
              out: 2,
              start: 3,
              opacity: 0.8,
              blend: "screen",
              transform: { scale: 1.2, rotation: 4 },
              effects: [{ type: "blur", params: { radius: 2 } }],
              keyframes: [{ property: "opacity", time: 0, value: 0 }],
              transitions: [{ type: "fade", duration: 0.25 }],
            },
          ],
          gaps: [{ timelineStart: 0, duration: 1 }],
        },
        {
          id: "t",
          type: "title",
          clips: [{ id: "title1", timelineStart: 0, duration: 2, text: "Hello" }],
        },
      ],
      markers: [{ id: "m", start: 1, label: "beat" }],
    });
    expect(parsed.tracks[0]!.type).toBe("video");
    expect(parsed.tracks[0]!.clips[0]!.timelineStart).toBe(3);
    expect(parsed.tracks[1]!.clips[0]!.duration).toBe(2);
    expect(parsed.markers[0]!.time).toBe(1);
  });

  it("migrates sequential v1 placement per track", () => {
    const migrated = migrateV1ToV2({
      version: 1,
      fps: 30,
      sources: [{ id: "s", path: "/tmp/s.mp4" }],
      tracks: [
        {
          id: "v0",
          clips: [
            { id: "a", src: "s", in: 0, out: 4, speed: 2 },
            { id: "b", src: "s", in: 4, out: 6 },
          ],
        },
      ],
      overlays: [],
    });
    expect(migrated.version).toBe(2);
    expect(migrated.tracks[0]!.type).toBe("video");
    expect(migrated.tracks[0]!.clips.map((clip) => clip.timelineStart)).toEqual([0, 2]);
    expect(migrated.tracks[0]!.clips[0]!.sourceId).toBe("s");
  });

  it("rejects conflicting aliases before emitting canonical fields", () => {
    expect(() =>
      TimelineClipSchema.parse({
        id: "c",
        sourceId: "a",
        src: "b",
        in: 0,
        out: 1,
        timelineStart: 0,
      }),
    ).toThrow(/source.*aliases.*agree/i);
    expect(() =>
      TimelineClipSchema.parse({
        id: "c",
        source: "a",
        assetId: "b",
        in: 0,
        out: 1,
        timelineStart: 0,
      }),
    ).toThrow(/source.*aliases.*agree/i);
    expect(() =>
      TimelineClipSchema.parse({
        id: "c",
        src: "a",
        in: 0,
        out: 1,
        timelineStart: 0,
        start: 1,
      }),
    ).toThrow(/timeline start.*aliases.*agree/i);
    expect(() =>
      TrackV2Schema.parse({ id: "t", type: "video", kind: "audio", clips: [] }),
    ).toThrow(/track type.*aliases.*agree/i);
    expect(() => GapSchema.parse({ start: 0, timelineStart: 1, duration: 1 })).toThrow(
      /gap start.*aliases.*agree/i,
    );
    expect(() => MarkerSchema.parse({ id: "m", time: 0, start: 1 })).toThrow(
      /marker time.*aliases.*agree/i,
    );
  });

  it("accepts agreeing aliases and emits canonical values", () => {
    const clip = TimelineClipSchema.parse({
      id: "c",
      src: "a",
      source: "a",
      assetId: "a",
      sourceIn: 0,
      sourceStart: 0,
      sourceOut: 1,
      sourceEnd: 1,
      start: 2,
    });
    expect(clip).toMatchObject({ sourceId: "a", in: 0, out: 1, timelineStart: 2 });
    expect(clip).not.toHaveProperty("src");
    expect(clip).not.toHaveProperty("start");
    expect(TrackV2Schema.parse({ id: "v", kind: "video", clips: [] })).toMatchObject({
      type: "video",
    });
    expect(GapSchema.parse({ timelineStart: 1, duration: 2 })).toEqual({
      start: 1,
      duration: 2,
    });
    expect(MarkerSchema.parse({ id: "m", start: 1 })).toEqual({ id: "m", time: 1 });
  });
});

describe("pure project history", () => {
  it("applies atomically and supports revision-checked undo/redo", () => {
    const initial = createProjectHistory(project());
    const applied = applyProjectCommands(
      initial,
      [
        {
          type: "add_clip",
          trackId: "titles",
          clip: { id: "title1", timelineStart: 0, duration: 2, text: "Hello" },
        },
        { type: "update_clip", clipId: "c1", patch: { opacity: 0.5 } },
      ],
      { expectedRevision: 0 },
    );
    expect(applied.revision).toBe(1);
    expect(applied.project.tracks[1]!.clips).toHaveLength(1);
    expect(applied.project.tracks[0]!.clips[0]!.opacity).toBe(0.5);
    expect(canUndo(applied)).toBe(true);
    expect(canRedo(applied)).toBe(false);

    const undone = undoProject(applied, { expectedRevision: 1 });
    expect(undone.revision).toBe(2);
    expect(undone.project).toEqual(initial.project);
    expect(canRedo(undone)).toBe(true);

    const redone = redoProject(undone, 2);
    expect(redone.revision).toBe(3);
    expect(redone.project).toEqual(applied.project);
  });

  it("does not mutate history on a failed command or revision conflict", () => {
    const initial = createProjectHistory(project());
    expect(() =>
      applyProjectCommand(initial, {
        type: "update_clip",
        clipId: "missing",
        patch: { opacity: 0 },
      }),
    ).toThrow(/not found/i);
    expect(initial.revision).toBe(0);
    expect(() =>
      applyProjectCommand(initial, {
        type: "update_clip",
        clipId: "c1",
        patch: { opacity: 0 },
      }, { expectedRevision: 1 }),
    ).toThrow(/revision/i);
    expect(initial.project.tracks[0]!.clips[0]!.opacity).toBeUndefined();
  });

  it("rejects empty batches without advancing revision", () => {
    const initial = createProjectHistory(project());
    expect(() => applyProjectCommands(initial, [])).toThrow(/empty/i);
    expect(initial.revision).toBe(0);
  });

  it("applies source and overlay commands and rejects replace_project", () => {
    const initial = project();
    const next = applyProject(initial, [
      { type: "add_source", source: { id: "cam-b", path: "/tmp/cam-b.mp4", duration: 30 } },
      {
        type: "add_overlay",
        overlay: { id: "subs-1", type: "subtitles", path: "/tmp/subs.srt" },
      },
    ]);
    expect(next.sources.map((source) => source.id)).toEqual(["src", "cam-b"]);
    expect(next.overlays).toMatchObject([{ id: "subs-1", type: "subtitles" }]);
    expect(ProjectSchema.safeParse(next).success).toBe(true);

    const removed = applyProject(next, { type: "remove_overlay", overlayId: "subs-1" });
    expect(removed.overlays).toHaveLength(0);

    expect(() =>
      applyProject(initial, { type: "add_source", source: { id: "src", path: "/tmp/dup.mp4" } }),
    ).toThrow(/already exists/i);
    expect(() =>
      applyProject(initial, {
        type: "add_overlay",
        overlay: { type: "subtitles", path: "/tmp/no-id.srt" },
      }),
    ).toThrow(/overlay\.id/i);
    expect(ProjectCommandSchema.safeParse({ type: "replace_project", project: initial }).success).toBe(
      false,
    );
  });
});
