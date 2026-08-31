import type { EditOp, Project, Timeline } from "@cut/timeline";
import type { MultimodalToolResult } from "./schema-bridge.js";

// ---------------------------------------------------------------------------
// Tool contracts for the @cut agent
// ---------------------------------------------------------------------------

/** A tool the agent can call. `input` is JSON-schema-ish (typed structurally). */
export interface Tool<I = unknown, O = unknown> {
  name: string;
  description: string;
  input: I;
  // eslint-disable-next-line @typescript-eslint/no-unused-vars
  output: O;
}

// --- fetch -----------------------------------------------------------------

export interface FetchInput {
  url: string;
}
export interface FetchOutput {
  path: string;
  subsPath?: string;
  title?: string;
}

// --- probe -----------------------------------------------------------------

export interface ProbeInput {
  action: "info" | "silence" | "scenes" | "frames";
  src: string;
  opts?: {
    /** for "frames": number of frames to extract (default 8, maximum 8) */
    frames?: number;
    /** for "silence": noise threshold in dB (default -30) */
    noiseDb?: number;
    /** for "silence": minimum silence duration in seconds (default 0.5) */
    minDuration?: number;
  };
}
export interface ProbeMetadataOutput {
  info: Record<string, unknown>;
  ranges?: Array<{ start: number; end: number }>;
  frames?: string[];
}
/** `action: "frames"` returns native images plus `{time, path}` metadata. */
export type ProbeOutput = ProbeMetadataOutput | MultimodalToolResult;

// --- edit ------------------------------------------------------------------

/**
 * A single edit or a non-empty atomic batch.  The mutually-exclusive shape is
 * deliberate: accepting both keys makes it unclear whether the caller meant
 * one operation or a transaction.
 */
export type EditInput =
  | { op: EditOp; ops?: never }
  | { ops: [EditOp, ...EditOp[]]; op?: never };
export type EditOutput = Timeline;

// --- canonical v2 project edit -------------------------------------------

/** Canonical v2 command envelope. The host performs full ProjectCommandSchema validation. */
export interface ProjectEditCommand {
  type: string;
  [key: string]: unknown;
}

/**
 * Canonical color-grading parameters carried by a v2 `color` effect.
 *
 * Every field is optional because omitted values use the renderer's identity
 * default.  The backend schema is intentionally closed; adding a control
 * requires updating the shared v2 contract rather than silently accepting an
 * unknown parameter.
 */
export interface ColorGradeParams {
  /** [-3, 3], identity 0. */
  exposure?: number;
  /** [0, 4], identity 1. */
  contrast?: number;
  /** [-1, 1], identity 0. */
  brightness?: number;
  /** [0, 4], identity 1. */
  saturation?: number;
  /** [-1000, 1000], identity 0. */
  temperature?: number;
  /** [-1, 1], identity 0. */
  tint?: number;
  /** [0.1, 10], identity 1. */
  gamma?: number;
  /** [-1, 1], identity 0. */
  highlights?: number;
  /** [-1, 1], identity 0. */
  shadows?: number;
  /** [-1, 1], identity 0. */
  vibrance?: number;
  /** [0, 1], identity 1. */
  mix?: number;
}

/** Structural v2 effect shape used by the agent-side command helper. */
export interface ProjectEffect {
  type: string;
  id?: string;
  enabled?: boolean;
  params?: unknown;
  [key: string]: unknown;
}

/** A color effect after normalization. */
export interface ColorEffect extends ProjectEffect {
  type: "color";
  params: ColorGradeParams;
}

/**
 * Input for `createColorGradeCommand`.
 *
 * Pass the clip's current `effects` from `timeline` when available.  The
 * helper keeps all non-color effects and merges a new color adjustment with
 * the existing color effect instead of replacing the entire effect stack.
 */
export interface ColorGradeInput {
  clipId: string;
  params: ColorGradeParams;
  effects?: readonly ProjectEffect[];
  existingEffects?: readonly ProjectEffect[];
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function withoutUndefined(value: object): Record<string, unknown> {
  return Object.fromEntries(Object.entries(value).filter(([, entry]) => entry !== undefined));
}

/**
 * Merge color parameters into an effect stack without mutating the input.
 * Existing duplicate color effects collapse to one normalized color effect;
 * all unrelated effects retain their order and fields.
 */
export function mergeColorGradeEffects(
  existingEffects: readonly ProjectEffect[] = [],
  params: ColorGradeParams = {},
): ProjectEffect[] {
  const normalizedParams = withoutUndefined(params);
  const merged: ProjectEffect[] = [];
  let colorInserted = false;

  for (const effect of existingEffects) {
    const copy: ProjectEffect = {
      ...effect,
      ...(isRecord(effect.params) ? { params: { ...effect.params } } : {}),
    };
    if (effect.type !== "color") {
      merged.push(copy);
      continue;
    }
    if (colorInserted) continue;
    colorInserted = true;
    merged.push({
      ...copy,
      type: "color",
      params: {
        ...(isRecord(effect.params) ? effect.params : {}),
        ...normalizedParams,
      },
    });
  }

  if (!colorInserted) {
    merged.push({ type: "color", params: normalizedParams });
  }
  return merged;
}

/**
 * Build the v2 `project_edit` command for clip color grading. Returns
 * `update_clip` instead of a parallel `set_color` command, so undo, revision
 * checks, and rendering stay on the existing project command path.
 */
export function createColorGradeCommand(input: ColorGradeInput): ProjectEditCommand {
  if (typeof input.clipId !== "string" || input.clipId.trim().length === 0) {
    throw new Error("color grading requires a non-empty clipId");
  }
  if (!isRecord(input.params)) {
    throw new Error("color grading params must be an object");
  }
  const effects = mergeColorGradeEffects(
    input.effects ?? input.existingEffects ?? [],
    input.params,
  );
  return {
    type: "update_clip",
    clipId: input.clipId,
    patch: { effects },
  };
}

/** JSON Schema for the normalized color parameters in a v2 color effect. */
export const COLOR_GRADE_PARAMS_JSON_SCHEMA: Record<string, unknown> = {
  type: "object",
  description:
    "Closed v2 color controls. Omit a control to use its identity value: exposure/brightness/temperature/tint/highlights/shadows/vibrance are 0; contrast/saturation/gamma/mix are 1.",
  properties: {
    exposure: { type: "number", minimum: -3, maximum: 3, default: 0, description: "Exposure; identity is 0." },
    contrast: { type: "number", minimum: 0, maximum: 4, default: 1, description: "Contrast multiplier; identity is 1." },
    brightness: { type: "number", minimum: -1, maximum: 1, default: 0, description: "Brightness; identity is 0." },
    saturation: { type: "number", minimum: 0, maximum: 4, default: 1, description: "Saturation multiplier; identity is 1." },
    temperature: { type: "number", minimum: -1000, maximum: 1000, default: 0, description: "White-balance temperature offset; identity is 0." },
    tint: { type: "number", minimum: -1, maximum: 1, default: 0, description: "Green/magenta tint; identity is 0." },
    gamma: { type: "number", minimum: 0.1, maximum: 10, default: 1, description: "Gamma multiplier; identity is 1." },
    highlights: { type: "number", minimum: -1, maximum: 1, default: 0, description: "Highlights adjustment; identity is 0." },
    shadows: { type: "number", minimum: -1, maximum: 1, default: 0, description: "Shadows adjustment; identity is 0." },
    vibrance: { type: "number", minimum: -1, maximum: 1, default: 0, description: "Vibrance adjustment; identity is 0." },
    mix: { type: "number", minimum: 0, maximum: 1, default: 1, description: "Grade mix; identity is 1." },
  },
  additionalProperties: false,
};

/** JSON Schema for one color effect in a clip's effect stack. */
export const COLOR_EFFECT_JSON_SCHEMA: Record<string, unknown> = {
  type: "object",
  description: "Normalized color-grade effect. Keep unrelated effects in the array.",
  properties: {
    type: { const: "color" },
    id: { type: "string", minLength: 1 },
    enabled: { type: "boolean" },
    params: COLOR_GRADE_PARAMS_JSON_SCHEMA,
  },
  required: ["type", "params"],
  additionalProperties: true,
};

/**
 * Input schema for `createColorGradeCommand` before `project_edit`.
 */
export const COLOR_GRADE_INPUT_JSON_SCHEMA: Record<string, unknown> = {
  type: "object",
  description:
    "Describe a clip color grade. Read the clip first and pass its existing effects so unrelated effects are preserved, then normalize to update_clip.patch.effects.",
  properties: {
    clipId: { type: "string", minLength: 1, description: "Clip id from timeline." },
    params: COLOR_GRADE_PARAMS_JSON_SCHEMA,
    effects: {
      type: "array",
      description: "Current clip effects from timeline; non-color entries are retained.",
      items: { type: "object" },
    },
    existingEffects: {
      type: "array",
      description: "Alias for effects when the current effect snapshot is named existingEffects.",
      items: { type: "object" },
    },
  },
  required: ["clipId", "params"],
  additionalProperties: false,
};

export interface ProjectEditInput {
  expectedRevision?: number;
  command?: ProjectEditCommand;
  commands?: [ProjectEditCommand, ...ProjectEditCommand[]];
}

export interface ProjectEditOutput {
  project: Project;
  revision: number;
  canUndo: boolean;
  canRedo: boolean;
}

// --- render ----------------------------------------------------------------

export interface RenderInput {
  /** draft mode: fast, lower-quality proxy render */
  draft?: boolean;
}
export interface RenderOutput {
  output: string;
}

// --- bash ------------------------------------------------------------------

export interface BashInput {
  command: string;
}
export interface BashOutput {
  stdout: string;
  stderr: string;
  exitCode: number;
}

// --- timeline -------------------------------------------------------------

/**
 * Read the current project timeline before making an edit. `include` is
 * optional: the tool always returns the validated timeline, and the value
 * lets callers make their intent explicit when requesting a compact or
 * complete summary.
 */
export interface TimelineInput {
  include?: "summary" | "full";
}

export interface TimelineClipSummary {
  id: string;
  trackId: string;
  src: string;
  /** Position and duration on the output track, in seconds. */
  position: number;
  duration: number;
  in: number;
  out: number;
  speed?: number;
}

export interface TimelineOutput {
  timeline: Timeline;
  summary: {
    duration: number;
    clips: TimelineClipSummary[];
  };
}

// --- optional vision ------------------------------------------------------

/** Input accepted by the optional, preconfigured object-tracking tool. */
export interface TrackObjectsInput {
  /** Media path relative to the project workspace (or an absolute path). */
  source: string;
  fps?: number;
  frameWidth?: number;
  frameHeight?: number;
  maxFrames?: number;
  /** Ask the configured planner for a normalized crop around tracked ROIs. */
  crop?: {
    aspectRatio: number;
    padding?: number;
  };
}

export interface TrackObjectsOutput {
  source: string;
  model: string;
  targets: readonly ["cam", "chat", "content"];
  detections: ReadonlyArray<{
    trackId: number | string;
    class: "cam" | "chat" | "content";
    confidence: number;
    frame: number;
    time: number;
    bbox: { x: number; y: number; width: number; height: number };
  }>;
  frameCount: number;
  fps?: number;
  plan: unknown;
  cropPlan?: unknown;
}

// --- registry --------------------------------------------------------------

export type CutTools = {
  fetch: Tool<FetchInput, FetchOutput>;
  probe: Tool<ProbeInput, ProbeOutput>;
  edit: Tool<EditInput, EditOutput>;
  render: Tool<RenderInput, RenderOutput>;
  bash?: Tool<BashInput, BashOutput>;
  timeline: Tool<TimelineInput, TimelineOutput>;
  project_edit?: Tool<ProjectEditInput, ProjectEditOutput>;
  track_objects?: Tool<TrackObjectsInput, TrackObjectsOutput>;
};

/** JSON Schema for the timeline/source fields exposed to edit operations. */
const sourceJsonSchema = {
  type: "object",
  description: "A local media source. Probe metadata should be copied when known.",
  properties: {
    id: { type: "string", description: "Unique source id used by clips." },
    path: { type: "string", description: "Media path, usually the fetched local path." },
    subs: { type: "string", description: "Optional subtitle file path for this source." },
    duration: { type: "number", minimum: 0, description: "Probed duration in seconds." },
    width: { type: "number", exclusiveMinimum: 0, description: "Probed video width in pixels." },
    height: { type: "number", exclusiveMinimum: 0, description: "Probed video height in pixels." },
    hasAudio: { type: "boolean", description: "Whether the probed source contains an audio stream." },
  },
  required: ["id", "path"],
  additionalProperties: false,
} as const;

const cropJsonSchema = {
  type: "object",
  description:
    "Normalized crop rectangle from a screenshot: x, y, width and height are all 0..1 fractions of the frame (origin at top-left).",
  properties: {
    x: { type: "number", minimum: 0, maximum: 1, description: "Left edge as a normalized frame fraction." },
    y: { type: "number", minimum: 0, maximum: 1, description: "Top edge as a normalized frame fraction." },
    width: { type: "number", exclusiveMinimum: 0, maximum: 1, description: "Crop width as a normalized frame fraction." },
    height: { type: "number", exclusiveMinimum: 0, maximum: 1, description: "Crop height as a normalized frame fraction." },
  },
  required: ["x", "y", "width", "height"],
  additionalProperties: false,
} as const;

const clipJsonSchema = {
  type: "object",
  description: "A source-time clip placed on a track.",
  properties: {
    id: { type: "string", description: "Unique clip id used by later edit operations." },
    src: { type: "string", description: "Source id." },
    in: { type: "number", minimum: 0, description: "Source start time in seconds." },
    out: { type: "number", exclusiveMinimum: 0, description: "Source end time in seconds; must be greater than in." },
    speed: { type: "number", exclusiveMinimum: 0, description: "Playback speed multiplier (default 1)." },
    volume: { type: "number", minimum: 0, description: "Linear volume (1 original, 0 muted)." },
    crop: cropJsonSchema,
  },
  required: ["id", "src", "in", "out"],
  additionalProperties: false,
} as const;

const captionCueJsonSchema = {
  type: "object",
  description: "One inline caption cue, timed in seconds on the output timeline.",
  properties: {
    start: { type: "number", minimum: 0, description: "Cue start time in output seconds." },
    end: { type: "number", exclusiveMinimum: 0, description: "Cue end time in output seconds; must be greater than start." },
    text: { type: "string", minLength: 1, description: "Non-empty caption text shown to the viewer." },
  },
  required: ["start", "end", "text"],
  additionalProperties: false,
} as const;

const subtitleStyleJsonSchema = {
  type: "object",
  properties: {
    fontSize: { type: "number", exclusiveMinimum: 0 },
    fontColor: { type: "string" },
    outlineColor: { type: "string" },
    position: { type: "string", enum: ["bottom", "center", "top"] },
    marginV: { type: "number" },
  },
  additionalProperties: false,
} as const;

const trimOpJsonSchema = {
  type: "object",
  properties: {
    op: { const: "trim" },
    clipId: { type: "string", description: "Clip id from timeline." },
    newIn: { type: "number", minimum: 0, description: "New source start in seconds." },
    newOut: { type: "number", exclusiveMinimum: 0, description: "New source end in seconds." },
  },
  required: ["op", "clipId"],
  additionalProperties: false,
} as const;

const splitOpJsonSchema = {
  type: "object",
  properties: {
    op: { const: "split" },
    clipId: { type: "string", description: "Clip id from timeline." },
    at: { type: "number", minimum: 0, description: "Absolute source time strictly inside the clip." },
  },
  required: ["op", "clipId", "at"],
  additionalProperties: false,
} as const;

const removeRangesOpJsonSchema = {
  type: "object",
  properties: {
    op: { const: "remove_ranges" },
    ranges: {
      type: "array",
      description: "Output-timeline ranges to remove; surviving clips are packed back-to-back.",
      items: {
        type: "object",
        properties: {
          start: { type: "number", minimum: 0 },
          end: { type: "number", exclusiveMinimum: 0 },
        },
        required: ["start", "end"],
        additionalProperties: false,
      },
    },
  },
  required: ["op", "ranges"],
  additionalProperties: false,
} as const;

const setSpeedOpJsonSchema = {
  type: "object",
  properties: {
    op: { const: "set_speed" },
    clipId: { type: "string" },
    speed: { type: "number", exclusiveMinimum: 0, description: "Playback multiplier; 2 is twice as fast." },
  },
  required: ["op", "clipId", "speed"],
  additionalProperties: false,
} as const;

const setVolumeOpJsonSchema = {
  type: "object",
  properties: {
    op: { const: "set_volume" },
    clipId: { type: "string" },
    volume: { type: "number", minimum: 0, description: "Linear volume (0 mutes, 1 original)." },
  },
  required: ["op", "clipId", "volume"],
  additionalProperties: false,
} as const;

const addSubtitlesOpJsonSchema = {
  type: "object",
  properties: {
    op: { const: "add_subtitles" },
    path: { type: "string", description: "Subtitle file to burn in (.srt/.vtt/.ass)." },
    style: subtitleStyleJsonSchema,
  },
  required: ["op", "path"],
  additionalProperties: false,
} as const;

const removeOverlayOpJsonSchema = {
  type: "object",
  properties: {
    op: { const: "remove_overlay" },
    id: { type: "string", description: "Overlay id from the timeline." },
  },
  required: ["op", "id"],
  additionalProperties: false,
} as const;

const addSourceOpJsonSchema = {
  type: "object",
  properties: {
    op: { const: "add_source" },
    source: {
      ...sourceJsonSchema,
      description: "Source metadata object; existing sources remain in the timeline.",
    },
  },
  required: ["op", "source"],
  additionalProperties: false,
  description: "Register another source without replacing existing sources.",
} as const;

const addClipOpJsonSchema = {
  type: "object",
  properties: {
    op: { const: "add_clip" },
    trackId: {
      type: "string",
      description: "Destination track id; defaults to the first video track.",
    },
    clip: {
      type: "object",
      description:
        "Source-time clip. For a 0.1-second montage, make adjacent clips whose output durations are 0.1 seconds.",
      properties: {
        id: { type: "string", description: "Optional unique clip id; generated when omitted." },
        src: { type: "string", description: "Source id." },
        in: { type: "number", description: "Source start time in seconds." },
        out: { type: "number", description: "Source end time in seconds; must be greater than in." },
        speed: { type: "number", exclusiveMinimum: 0, description: "Playback speed multiplier." },
        volume: { type: "number", minimum: 0, description: "Linear volume (1 original, 0 muted)." },
        crop: cropJsonSchema,
      },
      required: ["src", "in", "out"],
      additionalProperties: false,
    },
    index: {
      type: "integer",
      minimum: 0,
      description: "Optional zero-based insertion index in the destination track.",
    },
  },
  required: ["op", "clip"],
  additionalProperties: false,
  description:
    "Add a clip. For a 0.1-second montage, make adjacent clips whose output durations are 0.1 seconds.",
} as const;

const removeClipOpJsonSchema = {
  type: "object",
  properties: {
    op: { const: "remove_clip" },
    clipId: { type: "string", description: "Clip id to remove from its track." },
  },
  required: ["op", "clipId"],
  additionalProperties: false,
} as const;

const moveClipOpJsonSchema = {
  type: "object",
  properties: {
    op: { const: "move_clip" },
    clipId: { type: "string", description: "Clip id to move." },
    trackId: { type: "string", description: "Optional destination track id." },
    toIndex: { type: "integer", minimum: 0, description: "Zero-based insertion index in the destination track." },
  },
  required: ["op", "clipId", "toIndex"],
  additionalProperties: false,
  description: "Move/reorder a clip; use toIndex to build a precise montage order.",
} as const;

const setCropOpJsonSchema = {
  type: "object",
  properties: {
    op: { const: "set_crop" },
    clipId: { type: "string" },
    crop: {
      anyOf: [cropJsonSchema, { type: "null" }],
      description:
        "Normalized screenshot crop: x/y/width/height are 0..1 frame fractions, origin top-left. Use null to clear it.",
    },
  },
  required: ["op", "clipId", "crop"],
  additionalProperties: false,
  description:
    "Crop a screenshot-selected region for the whole clip using normalized 0..1 x/y/width/height fractions (origin top-left). " +
    "For a timed zoom, split at the source times first, then set_crop on the middle clip in the same atomic batch.",
} as const;

const addCaptionsOpJsonSchema = {
  type: "object",
  properties: {
    op: { const: "add_captions" },
    cues: {
      type: "array",
      minItems: 1,
      items: captionCueJsonSchema,
      description: "One or more inline caption cues; no sidecar file is needed.",
    },
    style: subtitleStyleJsonSchema,
  },
  required: ["op", "cues"],
  additionalProperties: false,
  description:
    "Add inline timed caption cues. Use start/end seconds and text; these cues are visible captions, not an external subtitle file.",
} as const;

/** Detailed EditOp union schema used by both provider adapters and CLI tools. */
export const EDIT_OP_JSON_SCHEMA: Record<string, unknown> = {
  oneOf: [
    trimOpJsonSchema,
    splitOpJsonSchema,
    removeRangesOpJsonSchema,
    setSpeedOpJsonSchema,
    setVolumeOpJsonSchema,
    addSubtitlesOpJsonSchema,
    removeOverlayOpJsonSchema,
    addSourceOpJsonSchema,
    addClipOpJsonSchema,
    removeClipOpJsonSchema,
    moveClipOpJsonSchema,
    setCropOpJsonSchema,
    addCaptionsOpJsonSchema,
  ],
  description:
    "One timeline edit operation. Use timeline first to learn source and clip ids.",
};

/** Exactly one single-op or non-empty batch edit request. */
export const EDIT_INPUT_JSON_SCHEMA: Record<string, unknown> = {
  type: "object",
  description:
    "Apply one edit or a non-empty atomic batch. Exactly one of op or ops is allowed; a failed batch leaves the timeline unchanged.",
  properties: {
    op: EDIT_OP_JSON_SCHEMA,
    ops: {
      type: "array",
      minItems: 1,
      items: EDIT_OP_JSON_SCHEMA,
      description:
        "Non-empty ordered edit batch applied atomically. Useful for joining many 0.1-second clips in one request.",
    },
  },
  oneOf: [
    { required: ["op"], not: { required: ["ops"] } },
    { required: ["ops"], not: { required: ["op"] } },
  ],
  additionalProperties: false,
};

const genericProjectEffectJsonSchema = {
  type: "object",
  description: "An existing non-color effect; preserve it when changing color grading.",
  properties: {
    type: { type: "string", minLength: 1 },
    params: { type: "object" },
  },
  required: ["type"],
  additionalProperties: true,
} as const;

const updateClipPatchJsonSchema = {
  type: "object",
  description:
    "Patch fields on a v2 clip. For color grading, include the full current effects array and add or replace one {type:'color', params:{...}} entry so unrelated effects are not discarded.",
  properties: {
    effects: {
      type: "array",
      description:
        "The clip effect stack. Color grading is represented by one color effect; retain other entries returned by timeline.",
      items: {
        oneOf: [COLOR_EFFECT_JSON_SCHEMA, genericProjectEffectJsonSchema],
      },
    },
  },
  additionalProperties: true,
} as const;

const PROJECT_COMMAND_JSON_SCHEMA: Record<string, unknown> = {
  type: "object",
  oneOf: [
    { type: "object", properties: { type: { const: "add_track" }, track: { type: "object" } }, required: ["type", "track"], additionalProperties: true },
    { type: "object", properties: { type: { const: "remove_track" }, trackId: { type: "string", minLength: 1 } }, required: ["type", "trackId"], additionalProperties: true },
    { type: "object", properties: { type: { const: "add_clip" }, trackId: { type: "string", minLength: 1 }, clip: { type: "object" }, index: { type: "integer", minimum: 0 } }, required: ["type", "trackId", "clip"], additionalProperties: true },
    { type: "object", properties: { type: { const: "remove_clip" }, clipId: { type: "string", minLength: 1 }, trackId: { type: "string", minLength: 1 } }, required: ["type", "clipId"], additionalProperties: true },
    { type: "object", properties: { type: { const: "move_clip" }, clipId: { type: "string", minLength: 1 }, trackId: { type: "string", minLength: 1 }, toTrackId: { type: "string", minLength: 1 }, index: { type: "integer", minimum: 0 }, toIndex: { type: "integer", minimum: 0 } }, required: ["type", "clipId"], oneOf: [{ required: ["index"] }, { required: ["toIndex"] }], additionalProperties: true },
    {
      type: "object",
      properties: {
        type: { const: "update_clip" },
        clipId: { type: "string", minLength: 1 },
        patch: updateClipPatchJsonSchema,
      },
      required: ["type", "clipId", "patch"],
      additionalProperties: true,
      description:
        "Update a clip. To color grade it, use patch.effects with a color effect and preserve unrelated existing effects.",
    },
    { type: "object", properties: { type: { const: "set_clip" }, clipId: { type: "string", minLength: 1 }, clip: { type: "object" } }, required: ["type", "clipId", "clip"], additionalProperties: true },
    { type: "object", properties: { type: { const: "add_marker" }, marker: { type: "object" } }, required: ["type", "marker"], additionalProperties: true },
    { type: "object", properties: { type: { const: "remove_marker" }, markerId: { type: "string", minLength: 1 } }, required: ["type", "markerId"], additionalProperties: true },
    { type: "object", properties: { type: { const: "set_metadata" }, patch: { type: "object" } }, required: ["type", "patch"], additionalProperties: true },
    { type: "object", properties: { type: { const: "add_source" }, source: { type: "object" } }, required: ["type", "source"], additionalProperties: true },
    { type: "object", properties: { type: { const: "add_overlay" }, overlay: { type: "object" } }, required: ["type", "overlay"], additionalProperties: true },
    { type: "object", properties: { type: { const: "remove_overlay" }, overlayId: { type: "string", minLength: 1 } }, required: ["type", "overlayId"], additionalProperties: true },
    { type: "object", properties: { type: { const: "batch" }, commands: { type: "array", minItems: 1, items: { type: "object", properties: { type: { type: "string", enum: ["add_track", "remove_track", "add_clip", "remove_clip", "move_clip", "update_clip", "set_clip", "add_marker", "remove_marker", "set_metadata", "add_source", "add_overlay", "remove_overlay", "batch"] } }, required: ["type"], additionalProperties: true } } }, required: ["type", "commands"], additionalProperties: true },
  ],
  additionalProperties: true,
};

/** Schema for a canonical v2 command or atomic command batch. */
export const PROJECT_EDIT_INPUT_JSON_SCHEMA: Record<string, unknown> = {
  type: "object",
  description:
    "Apply one canonical v2 project command or an atomic command batch. Use expectedRevision to reject stale edits. " +
    "For clip color grading, send update_clip with patch.effects containing {type:'color',params:{...}} and retain unrelated effects from timeline.",
  properties: {
    expectedRevision: { type: "integer", minimum: 0 },
    command: PROJECT_COMMAND_JSON_SCHEMA,
    commands: { type: "array", minItems: 1, items: PROJECT_COMMAND_JSON_SCHEMA },
  },
  oneOf: [
    { required: ["command"], not: { required: ["commands"] } },
    { required: ["commands"], not: { required: ["command"] } },
  ],
  examples: [
    {
      command: {
        type: "update_clip",
        clipId: "clip-1",
        patch: {
          effects: [
            {
              type: "color",
              params: { exposure: 0.25, contrast: 0.1, saturation: 0.05 },
            },
          ],
        },
      },
    },
  ],
  additionalProperties: false,
};

/** Schema for the read-only timeline inspection tool. */
export const TIMELINE_INPUT_JSON_SCHEMA: Record<string, unknown> = {
  type: "object",
  description:
    "Read the validated project timeline, source ids, clip ids, positions, and output durations before editing.",
  properties: {
    include: {
      type: "string",
      enum: ["summary", "full"],
      description: "Optional response detail hint; the validated timeline is always returned.",
    },
  },
  additionalProperties: false,
};

/** Schema contains no model/weights fields. Those are owned by
 * project/user configuration or an explicit BuildTools option. */
export const TRACK_OBJECTS_INPUT_JSON_SCHEMA: Record<string, unknown> = {
  type: "object",
  description:
    "Track configured cam, chat, and content objects. content is the main gameplay/stream canvas. Model weights and labels are fixed by project/user vision configuration.",
  properties: {
    source: { type: "string", minLength: 1, description: "Local media path." },
    fps: { type: "number", exclusiveMinimum: 0 },
    frameWidth: { type: "number", exclusiveMinimum: 0 },
    frameHeight: { type: "number", exclusiveMinimum: 0 },
    maxFrames: { type: "integer", minimum: 1 },
    crop: {
      type: "object",
      properties: {
        aspectRatio: { type: "number", exclusiveMinimum: 0 },
        padding: { type: "number", minimum: 0 },
      },
      required: ["aspectRatio"],
      additionalProperties: false,
    },
  },
  required: ["source"],
  additionalProperties: false,
};

/** Safe built-in tools advertised by default. */
export const TOOL_NAMES = ["fetch", "probe", "timeline", "project_edit", "render"] as const;
/** Optional tools are advertised only when their implementation is present. */
export const OPTIONAL_TOOL_NAMES = ["bash", "track_objects", "edit", "skill_read", "timeline_undo", "timeline_redo"] as const;
export type ToolName = (typeof TOOL_NAMES | typeof OPTIONAL_TOOL_NAMES)[number];
