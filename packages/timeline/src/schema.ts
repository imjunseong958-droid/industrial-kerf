import { z } from "zod";

// ---------------------------------------------------------------------------
// v1 timeline schema for @cut
// ---------------------------------------------------------------------------

export const SourceSchema = z.object({
  id: z.string(),
  path: z.string(),
  /** optional subtitle file path */
  subs: z.string().optional(),
  /** duration in seconds, when known from probing */
  duration: z.number().finite().nonnegative().optional(),
  /** video dimensions, when known from probing */
  width: z.number().finite().positive().optional(),
  height: z.number().finite().positive().optional(),
  /** whether the source has an audio stream, when known */
  hasAudio: z.boolean().optional(),
});
export type Source = z.infer<typeof SourceSchema>;

export const CropRegionSchema = z
  .object({
    /** normalized horizontal origin */
    x: z.number().finite().min(0).max(1),
    /** normalized vertical origin */
    y: z.number().finite().min(0).max(1),
    /** normalized width */
    width: z.number().finite().positive().max(1),
    /** normalized height */
    height: z.number().finite().positive().max(1),
  })
  .superRefine((crop, ctx) => {
    if (crop.x + crop.width > 1) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        path: ["width"],
        message: "crop x + width must be <= 1",
      });
    }
    if (crop.y + crop.height > 1) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        path: ["height"],
        message: "crop y + height must be <= 1",
      });
    }
  });
export type CropRegion = z.infer<typeof CropRegionSchema>;
export const CropSchema = CropRegionSchema;
export type Crop = CropRegion;

export const ClipSchema = z.object({
  /** unique within a timeline; used as EditOp target selector */
  id: z.string(),
  /** source id referencing sources[].id */
  src: z.string(),
  /** start time in seconds within the source */
  in: z.number(),
  /** end time in seconds within the source (> in) */
  out: z.number(),
  /** playback rate multiplier, default 1 */
  speed: z.number().finite().positive().optional(),
  /** linear volume, 1.0 = original, 0 = muted */
  volume: z.number().finite().min(0).optional(),
  /** optional normalized source crop region */
  crop: CropRegionSchema.optional(),
});
export type Clip = z.infer<typeof ClipSchema>;

export const TrackSchema = z.object({
  id: z.string(),
  clips: z.array(ClipSchema),
});
export type Track = z.infer<typeof TrackSchema>;

/** Overlays: typed loosely for now, extended later. */
export const OverlaySchema = z.object({
  /** Legacy generic overlays may omit an id; typed overlays require one. */
  id: z.string().optional(),
  kind: z.string(),
}).passthrough();
export type Overlay = z.infer<typeof OverlaySchema>;

export const SubtitleStyleSchema = z.object({
  fontSize: z.number().finite().positive().optional(),
  fontColor: z.string().optional(),
  outlineColor: z.string().optional(),
  position: z.enum(["bottom", "center", "top"]).optional(),
  marginV: z.number().finite().optional(),
});
export type SubtitleStyle = z.infer<typeof SubtitleStyleSchema>;

/** Burned-in subtitle overlay backed by an .srt/.ass file. */
export const SubtitlesOverlaySchema = z.object({
  id: z.string(),
  type: z.literal("subtitles"),
  path: z.string(),
  style: SubtitleStyleSchema.optional(),
});
export type SubtitlesOverlay = z.infer<typeof SubtitlesOverlaySchema>;

export const CaptionCueSchema = z
  .object({
    start: z.number().finite(),
    end: z.number().finite(),
    text: z.string().min(1),
  })
  .superRefine((cue, ctx) => {
    if (cue.start < 0) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        path: ["start"],
        message: "caption cue start must be nonnegative",
      });
    }
    if (cue.end < 0) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        path: ["end"],
        message: "caption cue end must be nonnegative",
      });
    }
    if (!(cue.end > cue.start)) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        path: [],
        message: "caption cue end must be greater than start",
      });
    }
  });
export type CaptionCue = z.infer<typeof CaptionCueSchema>;

/** Inline, timeline-relative captions (as opposed to a subtitle file). */
export const CaptionsOverlaySchema = z.object({
  id: z.string(),
  type: z.literal("captions"),
  cues: z.array(CaptionCueSchema).min(1),
  style: SubtitleStyleSchema.optional(),
});
export type CaptionsOverlay = z.infer<typeof CaptionsOverlaySchema>;

// Singular aliases for a single clip, source, or overlay.
export const CaptionOverlaySchema = CaptionsOverlaySchema;
export type CaptionOverlay = CaptionsOverlay;
export const InlineCaptionsOverlaySchema = CaptionsOverlaySchema;
export type InlineCaptionsOverlay = CaptionsOverlay;

export const TimelineSchema = z.object({
  version: z.literal(1),
  fps: z.number().finite().positive(),
  sources: z.array(SourceSchema),
  tracks: z.array(TrackSchema),
  overlays: z.array(z.union([SubtitlesOverlaySchema, CaptionsOverlaySchema, OverlaySchema])),
}).superRefine((timeline, ctx) => {
  const seenIds = (
    values: unknown[],
    path: (string | number)[],
    kind: string,
  ): Set<string> => {
    const ids = new Set<string>();
    values.forEach((value, index) => {
      const id =
        typeof value === "object" &&
        value !== null &&
        "id" in value &&
        typeof value.id === "string"
          ? value.id
          : undefined;
      if (id === undefined) return;
      if (ids.has(id)) {
        ctx.addIssue({
          code: z.ZodIssueCode.custom,
          path: [...path, index, "id"],
          message: `duplicate ${kind} id: ${id}`,
        });
      } else {
        ids.add(id);
      }
    });
    return ids;
  };

  const sourceIds = seenIds(timeline.sources, ["sources"], "source");
  seenIds(timeline.tracks, ["tracks"], "track");
  seenIds(timeline.overlays, ["overlays"], "overlay");

  const clipIds = new Set<string>();
  const sourcesById = new Map<string, Source>();
  timeline.sources.forEach((source, index) => {
    sourcesById.set(source.id, source);
    if (source.duration !== undefined && (!Number.isFinite(source.duration) || source.duration < 0)) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        path: ["sources", index, "duration"],
        message: "source duration must be finite and nonnegative",
      });
    }
  });

  timeline.tracks.forEach((track, trackIndex) => {
    track.clips.forEach((clip, clipIndex) => {
      if (clipIds.has(clip.id)) {
        ctx.addIssue({
          code: z.ZodIssueCode.custom,
          path: ["tracks", trackIndex, "clips", clipIndex, "id"],
          message: `duplicate clip id: ${clip.id}`,
        });
      } else {
        clipIds.add(clip.id);
      }

      if (!sourceIds.has(clip.src)) {
        ctx.addIssue({
          code: z.ZodIssueCode.custom,
          path: ["tracks", trackIndex, "clips", clipIndex, "src"],
          message: `clip references unknown source: ${clip.src}`,
        });
      }

      if (!Number.isFinite(clip.in) || clip.in < 0) {
        ctx.addIssue({
          code: z.ZodIssueCode.custom,
          path: ["tracks", trackIndex, "clips", clipIndex, "in"],
          message: "clip in must be finite and nonnegative",
        });
      }
      if (!Number.isFinite(clip.out) || clip.out < 0) {
        ctx.addIssue({
          code: z.ZodIssueCode.custom,
          path: ["tracks", trackIndex, "clips", clipIndex, "out"],
          message: "clip out must be finite and nonnegative",
        });
      }
      if (Number.isFinite(clip.in) && Number.isFinite(clip.out) && !(clip.out > clip.in)) {
        ctx.addIssue({
          code: z.ZodIssueCode.custom,
          path: ["tracks", trackIndex, "clips", clipIndex],
          message: "clip out must be greater than in",
        });
      }

      const source = sourcesById.get(clip.src);
      if (source?.duration !== undefined && Number.isFinite(clip.out) && clip.out > source.duration) {
        ctx.addIssue({
          code: z.ZodIssueCode.custom,
          path: ["tracks", trackIndex, "clips", clipIndex, "out"],
          message: `clip out exceeds source duration ${source.duration}`,
        });
      }

      if (clip.crop !== undefined) {
        const crop = clip.crop;
        if (
          !Number.isFinite(crop.x) ||
          !Number.isFinite(crop.y) ||
          !Number.isFinite(crop.width) ||
          !Number.isFinite(crop.height) ||
          crop.x < 0 ||
          crop.y < 0 ||
          crop.width < 0 ||
          crop.height < 0 ||
          crop.x > 1 ||
          crop.y > 1 ||
          crop.width > 1 ||
          crop.height > 1 ||
          crop.x + crop.width > 1 ||
          crop.y + crop.height > 1
        ) {
          ctx.addIssue({
            code: z.ZodIssueCode.custom,
            path: ["tracks", trackIndex, "clips", clipIndex, "crop"],
            message: "crop must be normalized and stay within [0, 1] bounds",
          });
        }
      }
    });
  });

  timeline.overlays.forEach((overlay, overlayIndex) => {
    if (
      (overlay as { type?: unknown }).type === "captions" &&
      "cues" in overlay &&
      Array.isArray(overlay.cues)
    ) {
      for (const [cueIndex, cue] of overlay.cues.entries()) {
        if (
          typeof cue !== "object" ||
          cue === null ||
          !("start" in cue) ||
          !("end" in cue) ||
          typeof cue.start !== "number" ||
          typeof cue.end !== "number"
        ) {
          ctx.addIssue({
            code: z.ZodIssueCode.custom,
            path: ["overlays", overlayIndex, "cues", cueIndex],
            message: "caption cue must contain numeric start and end values",
          });
          continue;
        }
        if (!Number.isFinite(cue.start) || cue.start < 0) {
          ctx.addIssue({
            code: z.ZodIssueCode.custom,
            path: ["overlays", overlayIndex, "cues", cueIndex, "start"],
            message: "caption cue start must be finite and nonnegative",
          });
        }
        if (!Number.isFinite(cue.end) || cue.end < 0) {
          ctx.addIssue({
            code: z.ZodIssueCode.custom,
            path: ["overlays", overlayIndex, "cues", cueIndex, "end"],
            message: "caption cue end must be finite and nonnegative",
          });
        }
        if (
          Number.isFinite(cue.start) &&
          Number.isFinite(cue.end) &&
          !(cue.end > cue.start)
        ) {
          ctx.addIssue({
            code: z.ZodIssueCode.custom,
            path: ["overlays", overlayIndex, "cues", cueIndex],
            message: "caption cue end must be greater than start",
          });
        }
      }
    }
  });
});
export type Timeline = z.infer<typeof TimelineSchema>;

// ---------------------------------------------------------------------------
// Edit operations
// ---------------------------------------------------------------------------

export const AddSubtitlesOpSchema = z.object({
  op: z.literal("add_subtitles"),
  path: z.string(),
  style: SubtitleStyleSchema.optional(),
});

export const AddSourceOpSchema = z.object({
  op: z.literal("add_source"),
  source: SourceSchema,
});

export const AddClipInputSchema = z.object({
  id: z.string().optional(),
  src: z.string(),
  in: z.number().finite(),
  out: z.number().finite(),
  speed: z.number().finite().positive().optional(),
  volume: z.number().finite().min(0).optional(),
  crop: CropRegionSchema.optional(),
});

export const AddClipOpSchema = z.object({
  op: z.literal("add_clip"),
  trackId: z.string().optional(),
  clip: AddClipInputSchema,
  index: z.number().int().nonnegative().optional(),
});

export const RemoveClipOpSchema = z.object({
  op: z.literal("remove_clip"),
  clipId: z.string(),
});

export const MoveClipOpSchema = z.object({
  op: z.literal("move_clip"),
  clipId: z.string(),
  toIndex: z.number().int().nonnegative(),
  trackId: z.string().optional(),
});

export const SetCropOpSchema = z.object({
  op: z.literal("set_crop"),
  clipId: z.string(),
  crop: CropRegionSchema.nullable(),
});

export const AddCaptionsOpSchema = z.object({
  op: z.literal("add_captions"),
  cues: z.array(CaptionCueSchema).min(1),
  style: SubtitleStyleSchema.optional(),
});

export const RemoveOverlayOpSchema = z.object({
  op: z.literal("remove_overlay"),
  id: z.string(),
});

export const TrimOpSchema = z.object({
  op: z.literal("trim"),
  /** clip id or track-scoped selector; identifies the target clip */
  clipId: z.string(),
  newIn: z.number().finite().optional(),
  newOut: z.number().finite().optional(),
});

export const SplitOpSchema = z.object({
  op: z.literal("split"),
  clipId: z.string(),
  /** time in seconds (within the clip's timeline position) where to cut */
  at: z.number().finite(),
});

export const RemoveRangesOpSchema = z.object({
  op: z.literal("remove_ranges"),
  ranges: z.array(z.object({ start: z.number().finite(), end: z.number().finite() })),
});

export const SetSpeedOpSchema = z.object({
  op: z.literal("set_speed"),
  clipId: z.string(),
  /** playback rate multiplier, e.g. 2 = double speed, 0.5 = half */
  speed: z.number().finite().positive(),
});

export const SetVolumeOpSchema = z.object({
  op: z.literal("set_volume"),
  clipId: z.string(),
  /** linear volume, 1.0 = original, 0 = muted */
  volume: z.number().finite().min(0),
});

export const EditOpSchema = z.discriminatedUnion("op", [
  AddSourceOpSchema,
  AddClipOpSchema,
  RemoveClipOpSchema,
  MoveClipOpSchema,
  SetCropOpSchema,
  AddCaptionsOpSchema,
  TrimOpSchema,
  SplitOpSchema,
  RemoveRangesOpSchema,
  SetSpeedOpSchema,
  SetVolumeOpSchema,
  AddSubtitlesOpSchema,
  RemoveOverlayOpSchema,
]);
export type EditOp = z.infer<typeof EditOpSchema>;
