import { TimelineSchema, type Clip, type Timeline } from "./schema.js";
import {
  ProjectSchema,
  type Project,
  type TrackType,
  type TimelineClip,
} from "./v2.js";
import { z } from "zod";

/** Parser that accepts v1 timelines and v2 projects. */
export const VersionedTimelineSchema: z.ZodTypeAny = z.union([TimelineSchema, ProjectSchema]);
export type VersionedTimeline = Timeline | Project;
export const AnyTimelineSchema: typeof VersionedTimelineSchema = VersionedTimelineSchema;

/**
 * Infer a useful v2 track type for a legacy track that had no discriminator.
 * Existing projects conventionally use `v0`/`v1` for video, while names that
 * explicitly mention audio, titles, or adjustments are preserved as such.
 */
function trackType(id: string): TrackType {
  const normalized = id.trim().toLowerCase();
  if (
    /^(a|audio)([-_]|$)/.test(normalized) ||
    /^(a|audio)\d+$/.test(normalized) ||
    normalized === "audio"
  ) {
    return "audio";
  }
  if (
    /^(title|titles)([-_]|$)/.test(normalized) ||
    /^(title|titles)\d+$/.test(normalized) ||
    normalized === "title"
  ) {
    return "title";
  }
  if (
    /^(adj|adjust|adjustment)([-_]|$)/.test(normalized) ||
    /^(adj|adjust|adjustment)\d+$/.test(normalized) ||
    normalized === "adjustment"
  ) {
    return "adjustment";
  }
  return "video";
}

function migrateClip(clip: Clip, timelineStart: number): TimelineClip {
  return {
    id: clip.id,
    sourceId: clip.src,
    in: clip.in,
    out: clip.out,
    timelineStart,
    ...(clip.speed !== undefined ? { speed: clip.speed } : {}),
    ...(clip.volume !== undefined ? { volume: clip.volume } : {}),
    ...(clip.crop !== undefined ? { crop: structuredClone(clip.crop) } : {}),
  };
}

/**
 * Migrate the sequential v1 timeline to an explicit-placement v2 project.
 *
 * v1 clip order defines placement independently for each track.  The output
 * start is therefore accumulated per track using the effective playback
 * duration `(out - in) / speed`; source intervals and IDs are preserved.  The
 * source references are written as `sourceId`, while the v1 `src` spelling is
 * still accepted by the v2 parser for boundary compatibility.
 */
export function migrateV1ToV2(input: Timeline | unknown): Project {
  const legacy = TimelineSchema.parse(input);
  const project: Project = {
    version: 2,
    fps: legacy.fps,
    sources: legacy.sources.map((source) => structuredClone(source)),
    tracks: legacy.tracks.map((track) => {
      let timelineStart = 0;
      const clips = track.clips.map((clip) => {
        const migrated = migrateClip(clip, timelineStart);
        timelineStart += (clip.out - clip.in) / (clip.speed ?? 1);
        return migrated;
      });
      return { id: track.id, type: trackType(track.id), clips };
    }),
    overlays: legacy.overlays.map((overlay) => structuredClone(overlay)),
    gaps: [],
    transitions: [],
    effects: [],
    markers: [],
  };
  return ProjectSchema.parse(project);
}

/** Parse and validate either a legacy timeline or a v2 project. */
export function parseTimelineProject(input: unknown): Timeline | Project {
  if (
    typeof input === "object" &&
    input !== null &&
    "version" in input &&
    (input as { version?: unknown }).version === 2
  ) {
    return ProjectSchema.parse(input);
  }
  return TimelineSchema.parse(input);
}

export function validateV1(input: unknown): Timeline {
  return TimelineSchema.parse(input);
}

export function validateV2(input: unknown): Project {
  return ProjectSchema.parse(input);
}
