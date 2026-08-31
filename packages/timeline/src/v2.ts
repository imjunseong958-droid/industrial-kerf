import { z } from "zod";

/**
 * The version 2 project IR.
 *
 * Version 1 lives in `schema.ts`. The media package and existing callers still
 * consume the compact, sequential timeline there.
 * This module is the non-rendering IR foundation. It has explicit output
 * placement (`timelineStart`) and is extensible at the leaves so new editor
 * features do not require a schema-version bump for every field.
 */

const finite = z.number().finite();
const nonnegative = finite.nonnegative();
const positive = finite.positive();

/** Reject contradictory spellings before an input is canonicalized. */
function assertAliasAgreement(
  value: Record<string, unknown>,
  keys: readonly string[],
  label: string,
): void {
  let firstKey: string | undefined;
  let firstValue: unknown;
  for (const key of keys) {
    if (value[key] === undefined) continue;
    if (firstKey === undefined) {
      firstKey = key;
      firstValue = value[key];
      continue;
    }
    if (!Object.is(firstValue, value[key])) {
      throw new Error(`${label} aliases must agree (${firstKey} conflicts with ${key})`);
    }
  }
}

export const TrackTypeSchema = z.enum(["video", "audio", "title", "adjustment"]);
export type TrackType = z.infer<typeof TrackTypeSchema>;

// Alias for TrackTypeSchema when tracks use `kind` instead of `type`.
export const TrackKindSchema = TrackTypeSchema;
export type TrackKind = TrackType;

export const BlendModeSchema = z.string().min(1);
export type BlendMode = z.infer<typeof BlendModeSchema>;

export const TransformSchema = z
  .object({
    x: finite.optional(),
    y: finite.optional(),
    z: finite.optional(),
    scale: positive.optional(),
    scaleX: positive.optional(),
    scaleY: positive.optional(),
    rotation: finite.optional(),
    rotationX: finite.optional(),
    rotationY: finite.optional(),
    anchorX: finite.optional(),
    anchorY: finite.optional(),
    position: z
      .union([
        z.object({ x: finite, y: finite, z: finite.optional() }).passthrough(),
        z.tuple([finite, finite]),
      ])
      .optional(),
    anchor: z
      .union([z.object({ x: finite, y: finite }).passthrough(), z.tuple([finite, finite])])
      .optional(),
  })
  .passthrough();
export type Transform = z.infer<typeof TransformSchema>;

const colorNumber = (min: number, max: number, identity: number) =>
  z.number().finite().min(min).max(max).default(identity);

/**
 * The serialisable color-grading controls understood by the v2 renderer.
 *
 * `temperature` is a signed Kelvin offset (bounded to a practical +/-1000K)
 * while the other signed controls use normalized [-1, 1] values.  `mix` is
 * the strength of the grade: 0 is the identity and 1 applies the full grade.
 * The schema is passthrough so newer clients can persist additional controls
 * without making old projects unreadable; renderers still reject controls
 * they do not know how to compile rather than silently dropping them.
 */
export const ColorEffectParamsSchema = z
  .object({
    exposure: colorNumber(-3, 3, 0),
    contrast: colorNumber(0, 4, 1),
    brightness: colorNumber(-1, 1, 0),
    saturation: colorNumber(0, 4, 1),
    temperature: colorNumber(-1000, 1000, 0),
    tint: colorNumber(-1, 1, 0),
    gamma: colorNumber(0.1, 10, 1),
    highlights: colorNumber(-1, 1, 0),
    shadows: colorNumber(-1, 1, 0),
    vibrance: colorNumber(-1, 1, 0),
    mix: colorNumber(0, 1, 1),
  })
  .passthrough();
export type ColorEffectParams = z.infer<typeof ColorEffectParamsSchema>;

export const ColorEffectSchema = z
  .object({
    id: z.string().min(1).optional(),
    type: z.literal("color"),
    enabled: z.boolean().optional(),
    params: ColorEffectParamsSchema.default({}),
  })
  .passthrough();
export type ColorEffect = z.infer<typeof ColorEffectSchema>;

const GenericEffectSchema = z
  .object({
    id: z.string().min(1).optional(),
    // A color effect has a typed branch above.  Excluding it here prevents an
    // invalid color payload from falling through this permissive legacy branch.
    type: z
      .string()
      .min(1)
      .refine((value) => value.toLowerCase() !== "color", {
        message: "color effects must use ColorEffectParamsSchema",
      }),
    enabled: z.boolean().optional(),
    params: z.record(z.unknown()).optional(),
  })
  .passthrough();

/** Generic effects remain accepted for forward/backward project compatibility. */
export const EffectSchema = z.union([ColorEffectSchema, GenericEffectSchema]);
export type Effect = z.infer<typeof EffectSchema>;

export const KeyframeSchema = z
  .object({
    id: z.string().min(1).optional(),
    property: z.string().min(1),
    /** Time relative to the containing clip unless a client says otherwise. */
    time: nonnegative,
    value: z.unknown(),
    easing: z.string().min(1).optional(),
  })
  .passthrough();
export type Keyframe = z.infer<typeof KeyframeSchema>;

export const TransitionSchema = z
  .object({
    id: z.string().min(1).optional(),
    type: z.string().min(1),
    duration: positive,
    alignment: z.enum(["in", "out", "center"]).optional(),
    params: z.record(z.unknown()).optional(),
  })
  .passthrough();
export type Transition = z.infer<typeof TransitionSchema>;

const GapInputSchema = z
  .object({
    id: z.string().min(1).optional(),
    start: nonnegative.optional(),
    timelineStart: nonnegative.optional(),
    duration: positive,
    label: z.string().optional(),
  })
  .passthrough();

/** An intentional empty interval on a track. */
export const GapSchema = z.preprocess((value) => {
  if (typeof value !== "object" || value === null) return value;
  const raw = value as Record<string, unknown>;
  assertAliasAgreement(raw, ["start", "timelineStart"], "gap start");
  const normalized = { ...raw };
  if (normalized.start === undefined && normalized.timelineStart !== undefined) {
    normalized.start = normalized.timelineStart;
  }
  delete normalized.timelineStart;
  return normalized;
}, GapInputSchema.superRefine((gap, ctx) => {
  if (gap.start === undefined) {
    ctx.addIssue({
      code: z.ZodIssueCode.custom,
      path: ["start"],
      message: "gap requires start or timelineStart",
    });
  }
}));
export type Gap = z.infer<typeof GapSchema>;

const MarkerInputSchema = z
  .object({
    id: z.string().min(1),
    time: nonnegative.optional(),
    start: nonnegative.optional(),
    label: z.string().optional(),
    color: z.string().optional(),
    duration: nonnegative.optional(),
    metadata: z.record(z.unknown()).optional(),
  })
  .passthrough();

/** A named, timeline-relative point or range. */
export const MarkerSchema = z.preprocess((value) => {
  if (typeof value !== "object" || value === null) return value;
  const raw = value as Record<string, unknown>;
  assertAliasAgreement(raw, ["time", "start"], "marker time");
  const normalized = { ...raw };
  if (normalized.time === undefined && normalized.start !== undefined) {
    normalized.time = normalized.start;
  }
  delete normalized.start;
  return normalized;
}, MarkerInputSchema.superRefine((marker, ctx) => {
  if (marker.time === undefined) {
    ctx.addIssue({
      code: z.ZodIssueCode.custom,
      path: ["time"],
      message: "marker requires time or start",
    });
  }
}));
export type Marker = z.infer<typeof MarkerSchema>;

const V2SourceSchema = z
  .object({
    id: z.string().min(1),
    path: z.string(),
    duration: nonnegative.optional(),
    width: positive.optional(),
    height: positive.optional(),
    hasAudio: z.boolean().optional(),
    metadata: z.record(z.unknown()).optional(),
  })
  .passthrough();
export const ProjectSourceSchema = V2SourceSchema;
export type ProjectSource = z.infer<typeof ProjectSourceSchema>;
export const SourceV2Schema = ProjectSourceSchema;
export type SourceV2 = ProjectSource;
export const SourceSchemaV2 = ProjectSourceSchema;

const V2ClipInputSchema = z
  .object({
    id: z.string().min(1),
    /** Canonical source reference. `src` is accepted as a v1-style alias. */
    sourceId: z.string().min(1).optional(),
    src: z.string().min(1).optional(),
    source: z.string().min(1).optional(),
    assetId: z.string().min(1).optional(),
    /** Source interval. Title and adjustment clips may omit it. */
    in: nonnegative.optional(),
    out: nonnegative.optional(),
    sourceIn: nonnegative.optional(),
    sourceOut: nonnegative.optional(),
    sourceStart: nonnegative.optional(),
    sourceEnd: nonnegative.optional(),
    /** Canonical explicit output placement. `start` is accepted as an alias. */
    timelineStart: nonnegative.optional(),
    start: nonnegative.optional(),
    /** Duration is useful for source-less title/adjustment clips. */
    duration: positive.optional(),
    speed: positive.optional(),
    volume: nonnegative.optional(),
    opacity: finite.min(0).max(1).optional(),
    transform: TransformSchema.optional(),
    blend: BlendModeSchema.optional(),
    blendMode: BlendModeSchema.optional(),
    effects: z.array(EffectSchema).optional(),
    keyframes: z.array(KeyframeSchema).optional(),
    transitions: z.array(TransitionSchema).optional(),
    gaps: z.array(GapSchema).optional(),
    text: z.string().optional(),
    title: z.union([z.string(), z.record(z.unknown())]).optional(),
  })
  .passthrough();

/**
 * A v2 clip. `timelineStart` is explicit; source-less title and adjustment
 * clips use `duration` and can carry `text`/effects instead of a source.
 *
 * Preprocessing accepts aliases. Parsed values emit the canonical
 * `timelineStart`/`sourceId` fields when their aliases were supplied.
 */
export const TimelineClipSchema = z.preprocess((value) => {
  if (typeof value !== "object" || value === null) return value;
  const raw = value as Record<string, unknown>;
  assertAliasAgreement(raw, ["sourceId", "src", "source", "assetId"], "clip source");
  assertAliasAgreement(raw, ["in", "sourceIn", "sourceStart"], "clip source in");
  assertAliasAgreement(raw, ["out", "sourceOut", "sourceEnd"], "clip source out");
  assertAliasAgreement(raw, ["timelineStart", "start"], "clip timeline start");
  const normalized = { ...raw };
  if (normalized.sourceId === undefined) {
    normalized.sourceId = normalized.src ?? normalized.source ?? normalized.assetId;
  }
  if (normalized.in === undefined) {
    normalized.in = normalized.sourceIn ?? normalized.sourceStart;
  }
  if (normalized.out === undefined) {
    normalized.out = normalized.sourceOut ?? normalized.sourceEnd;
  }
  if (normalized.timelineStart === undefined && normalized.start !== undefined) {
    normalized.timelineStart = normalized.start;
  }
  delete normalized.src;
  delete normalized.source;
  delete normalized.assetId;
  delete normalized.sourceIn;
  delete normalized.sourceOut;
  delete normalized.sourceStart;
  delete normalized.sourceEnd;
  delete normalized.start;
  return normalized;
}, V2ClipInputSchema.superRefine((clip, ctx) => {
  if (clip.timelineStart === undefined) {
    ctx.addIssue({
      code: z.ZodIssueCode.custom,
      path: ["timelineStart"],
      message: "clip requires timelineStart or start",
    });
  }

  const hasIn = clip.in !== undefined;
  const hasOut = clip.out !== undefined;
  if (hasIn !== hasOut) {
    ctx.addIssue({
      code: z.ZodIssueCode.custom,
      path: [hasIn ? "out" : "in"],
      message: "clip source interval requires both in and out",
    });
  }
  if (clip.in !== undefined && clip.out !== undefined && !(clip.out > clip.in)) {
    ctx.addIssue({
      code: z.ZodIssueCode.custom,
      path: ["out"],
      message: "clip out must be greater than in",
    });
  }
  if (clip.duration !== undefined && !(clip.duration > 0)) {
    ctx.addIssue({
      code: z.ZodIssueCode.custom,
      path: ["duration"],
      message: "clip duration must be positive",
    });
  }
}));
export type TimelineClip = z.infer<typeof TimelineClipSchema>;
export type ClipV2 = TimelineClip;
export const ClipV2Schema = TimelineClipSchema;
export const TimelineClipV2Schema = TimelineClipSchema;

const V2TrackInputSchema = z
  .object({
    id: z.string().min(1),
    /** Canonical discriminator. `kind` is accepted as an alias. */
    type: TrackTypeSchema.optional(),
    kind: TrackKindSchema.optional(),
    name: z.string().optional(),
    enabled: z.boolean().optional(),
    muted: z.boolean().optional(),
    locked: z.boolean().optional(),
    clips: z.array(TimelineClipSchema),
    gaps: z.array(GapSchema).optional(),
    transitions: z.array(TransitionSchema).optional(),
    effects: z.array(EffectSchema).optional(),
    markers: z.array(MarkerSchema).optional(),
  })
  .passthrough();

export const TrackV2Schema = z.preprocess((value) => {
  if (typeof value !== "object" || value === null) return value;
  const raw = value as Record<string, unknown>;
  assertAliasAgreement(raw, ["type", "kind"], "track type");
  const normalized = { ...raw };
  if (normalized.type === undefined && normalized.kind !== undefined) {
    normalized.type = normalized.kind;
  }
  delete normalized.kind;
  return normalized;
}, V2TrackInputSchema.superRefine((track, ctx) => {
  if (track.type === undefined) {
    ctx.addIssue({
      code: z.ZodIssueCode.custom,
      path: ["type"],
      message: "v2 track requires type or kind",
    });
  }
}));
export type TrackV2 = z.infer<typeof TrackV2Schema>;
export const TrackSchemaV2 = TrackV2Schema;

const OverlayInputSchema = z
  .object({
    id: z.string().min(1).optional(),
    type: z.string().min(1).optional(),
    kind: z.string().min(1).optional(),
    start: nonnegative.optional(),
    duration: positive.optional(),
  })
  .passthrough();

export const OverlayV2Schema = z.preprocess((value) => {
  if (typeof value !== "object" || value === null) return value;
  const raw = value as Record<string, unknown>;
  assertAliasAgreement(raw, ["type", "kind"], "overlay type");
  const normalized = { ...raw };
  if (normalized.type === undefined && normalized.kind !== undefined) {
    normalized.type = normalized.kind;
  }
  delete normalized.kind;
  return normalized;
}, OverlayInputSchema);
export type OverlayV2 = z.infer<typeof OverlayV2Schema>;

/** Canonical parsed v2 project shape.  Extension keys are allowed by the schema. */
export interface Project {
  version: 2;
  id?: string;
  name?: string;
  fps: number;
  width?: number;
  height?: number;
  duration?: number;
  sources: ProjectSource[];
  tracks: TrackV2[];
  overlays: OverlayV2[];
  gaps: Gap[];
  transitions: Transition[];
  effects: Effect[];
  markers: Marker[];
  metadata?: Record<string, unknown>;
  [key: string]: unknown;
}

const ProjectInputSchema: z.ZodTypeAny = z
  .object({
    version: z.literal(2),
    id: z.string().min(1).optional(),
    name: z.string().optional(),
    fps: positive,
    width: positive.optional(),
    height: positive.optional(),
    duration: nonnegative.optional(),
    sources: z.array(ProjectSourceSchema),
    tracks: z.array(TrackV2Schema),
    overlays: z.array(OverlayV2Schema).default([]),
    gaps: z.array(GapSchema).default([]),
    transitions: z.array(TransitionSchema).default([]),
    effects: z.array(EffectSchema).default([]),
    markers: z.array(MarkerSchema).default([]),
    metadata: z.record(z.unknown()).optional(),
  })
  .passthrough();

/** Validate project-wide references and identity constraints after field parsing. */
function addProjectIssues(project: Project, ctx: z.RefinementCtx): void {
  const uniqueIds = (
    values: Array<{ id?: string | undefined }>,
    path: (string | number)[],
    label: string,
  ): Set<string> => {
    const ids = new Set<string>();
    values.forEach((value, index) => {
      if (value.id === undefined) return;
      if (ids.has(value.id)) {
        ctx.addIssue({
          code: z.ZodIssueCode.custom,
          path: [...path, index, "id"],
          message: `duplicate ${label} id: ${value.id}`,
        });
      }
      ids.add(value.id);
    });
    return ids;
  };

  const sourceIds = uniqueIds(project.sources, ["sources"], "source");
  uniqueIds(project.tracks, ["tracks"], "track");
  uniqueIds(project.overlays, ["overlays"], "overlay");
  uniqueIds(project.markers, ["markers"], "marker");

  const clipIds = new Set<string>();
  project.tracks.forEach((track, trackIndex) => {
    track.clips.forEach((clip, clipIndex) => {
      if (clipIds.has(clip.id)) {
        ctx.addIssue({
          code: z.ZodIssueCode.custom,
          path: ["tracks", trackIndex, "clips", clipIndex, "id"],
          message: `duplicate clip id: ${clip.id}`,
        });
      }
      clipIds.add(clip.id);

      const sourceId = clip.sourceId;
      const trackType = track.type;
      if ((trackType === "video" || trackType === "audio") && sourceId === undefined) {
        ctx.addIssue({
          code: z.ZodIssueCode.custom,
          path: ["tracks", trackIndex, "clips", clipIndex, "sourceId"],
          message: `${trackType} clip requires a sourceId`,
        });
      }
      if (sourceId !== undefined && !sourceIds.has(sourceId)) {
        ctx.addIssue({
          code: z.ZodIssueCode.custom,
          path: ["tracks", trackIndex, "clips", clipIndex, "sourceId"],
          message: `clip references unknown source: ${sourceId}`,
        });
      }

      if (clip.in !== undefined && clip.out !== undefined && sourceId !== undefined) {
        const source = project.sources.find((candidate) => candidate.id === sourceId);
        if (source?.duration !== undefined && clip.out > source.duration) {
          ctx.addIssue({
            code: z.ZodIssueCode.custom,
            path: ["tracks", trackIndex, "clips", clipIndex, "out"],
            message: `clip out exceeds source duration ${source.duration}`,
          });
        }
      }
      if (
        sourceId === undefined &&
        clip.duration === undefined &&
        !(clip.in !== undefined && clip.out !== undefined)
      ) {
        ctx.addIssue({
          code: z.ZodIssueCode.custom,
          path: ["tracks", trackIndex, "clips", clipIndex, "duration"],
          message: "source-less clip requires duration",
        });
      }
    });
  });
}

export const ProjectSchema: z.ZodType<Project> = ProjectInputSchema.superRefine(
  addProjectIssues,
) as z.ZodType<Project>;

/** Optional document wrapper for hosts that keep project metadata beside the timeline. */
export const ProjectDocumentSchema: z.ZodTypeAny = z
  .object({
    version: z.literal(2),
    timeline: ProjectSchema,
    id: z.string().min(1).optional(),
    name: z.string().optional(),
    metadata: z.record(z.unknown()).optional(),
  })
  .passthrough();
export type ProjectDocument = {
  version: 2;
  timeline: Project;
  id?: string;
  name?: string;
  metadata?: Record<string, unknown>;
};

// Public aliases make the intended relationship explicit to callers while
// leaving the v1 `Timeline` name untouched.
export const TimelineV2Schema: typeof ProjectSchema = ProjectSchema;
export type TimelineV2 = Project;
export const ProjectV2Schema: typeof ProjectSchema = ProjectSchema;
export type ProjectV2 = Project;
export const TimelineSchemaV2: typeof ProjectSchema = ProjectSchema;

export function emptyProject(): Project {
  return ProjectSchema.parse({
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
}

export const createEmptyProject = emptyProject;

export function parseProject(value: unknown): Project {
  return ProjectSchema.parse(value);
}

export const parseTimelineV2 = parseProject;
export const validateProject = parseProject;
export const validateTimelineV2 = parseProject;

export function isProject(value: unknown): value is Project {
  return ProjectSchema.safeParse(value).success;
}

export const isTimelineV2 = isProject;

/** Return the canonical explicit start of a v2 clip. */
export function clipTimelineStart(clip: TimelineClip): number {
  return clip.timelineStart ?? 0;
}

/** Return a clip's canonical source id. */
export function clipSourceId(clip: TimelineClip): string | undefined {
  return clip.sourceId;
}

/** Return the output duration represented by a v2 clip where it is derivable. */
export function clipTimelineDuration(clip: TimelineClip): number | undefined {
  if (clip.duration !== undefined) return clip.duration;
  if (clip.in === undefined || clip.out === undefined) return undefined;
  return (clip.out - clip.in) / (clip.speed ?? 1);
}
