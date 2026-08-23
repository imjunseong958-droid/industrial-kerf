import { z } from "zod";

// ---------------------------------------------------------------------------
// Timeline — single source of truth for @cut
// ---------------------------------------------------------------------------

export const SourceSchema = z.object({
  id: z.string(),
  path: z.string(),
  /** optional subtitle file path */
  subs: z.string().optional(),
});
export type Source = z.infer<typeof SourceSchema>;

export const ClipSchema = z.object({
  /** source id referencing sources[].id */
  src: z.string(),
  /** start time in seconds within the source */
  in: z.number(),
  /** end time in seconds within the source (> in) */
  out: z.number(),
});
export type Clip = z.infer<typeof ClipSchema>;

export const TrackSchema = z.object({
  id: z.string(),
  clips: z.array(ClipSchema),
});
export type Track = z.infer<typeof TrackSchema>;

/** Overlays: typed loosely for now, extended later. */
export const OverlaySchema = z.object({
  kind: z.string(),
}).passthrough();
export type Overlay = z.infer<typeof OverlaySchema>;

export const TimelineSchema = z.object({
  version: z.literal(1),
  fps: z.number().positive(),
  sources: z.array(SourceSchema),
  tracks: z.array(TrackSchema),
  overlays: z.array(OverlaySchema),
});
export type Timeline = z.infer<typeof TimelineSchema>;

// ---------------------------------------------------------------------------
// Edit operations — the payload of the `edit` tool
// ---------------------------------------------------------------------------

export const TrimOpSchema = z.object({
  op: z.literal("trim"),
  /** clip id or track-scoped selector; identifies the target clip */
  clipId: z.string(),
  newIn: z.number().optional(),
  newOut: z.number().optional(),
});

export const SplitOpSchema = z.object({
  op: z.literal("split"),
  clipId: z.string(),
  /** time in seconds (within the clip's timeline position) where to cut */
  at: z.number(),
});

export const RemoveRangesOpSchema = z.object({
  op: z.literal("remove_ranges"),
  ranges: z.array(z.object({ start: z.number(), end: z.number() })),
});

export const SetSpeedOpSchema = z.object({
  op: z.literal("set_speed"),
  clipId: z.string(),
  /** playback rate multiplier, e.g. 2 = double speed, 0.5 = half */
  speed: z.number().positive(),
});

export const SetVolumeOpSchema = z.object({
  op: z.literal("set_volume"),
  clipId: z.string(),
  /** linear volume, 1.0 = original, 0 = muted */
  volume: z.number().min(0),
});

export const EditOpSchema = z.discriminatedUnion("op", [
  TrimOpSchema,
  SplitOpSchema,
  RemoveRangesOpSchema,
  SetSpeedOpSchema,
  SetVolumeOpSchema,
]);
export type EditOp = z.infer<typeof EditOpSchema>;
