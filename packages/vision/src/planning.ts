import { CutEngineError, invokeCutEngineJson } from "@cut/engine";
import { VISION_TARGETS } from "./types.js";
import type {
  NormalizedBoundingBox,
  NormalizedDetection,
  NormalizedTrackingResult,
  VisionTarget,
} from "./types.js";

/** An ROI keyframe suitable for a renderer or a later timeline operation. */
export interface RoiKeyframe {
  readonly segmentId: string;
  readonly trackId: number | string;
  readonly class: VisionTarget;
  readonly frame: number;
  readonly time: number;
  readonly confidence: number;
  readonly bbox: NormalizedBoundingBox;
}

/** A contiguous observation run for one semantic class and track ID. */
export interface TrackingSegment {
  readonly id: string;
  readonly trackId: number | string;
  readonly class: VisionTarget;
  readonly startTime: number;
  readonly endTime: number;
  readonly keyframes: readonly RoiKeyframe[];
}

export interface TrackingPlan {
  /** Stable, time-ordered segments across all semantic classes. */
  readonly segments: readonly TrackingSegment[];
  /** Explicit class separation so consumers cannot accidentally mix ROIs. */
  readonly byClass: Readonly<{
    readonly cam: readonly TrackingSegment[];
    readonly chat: readonly TrackingSegment[];
    readonly content: readonly TrackingSegment[];
  }>;
  readonly keyframes: readonly RoiKeyframe[];
  readonly keyframesByClass: Readonly<{
    readonly cam: readonly RoiKeyframe[];
    readonly chat: readonly RoiKeyframe[];
    readonly content: readonly RoiKeyframe[];
  }>;
}

export interface TrackingPlanOptions {
  /** Drop detections below this confidence before grouping. Default: 0.25. */
  readonly minConfidence?: number;
  readonly confidenceThreshold?: number;
  /** Maximum time gap within one segment. Default: 0.5 seconds. */
  readonly maxGapSeconds?: number;
  readonly maxGap?: number;
  /** EMA weight for the current bbox. 1 means no smoothing; default: 0.5. */
  readonly smoothingFactor?: number;
  readonly smoothingAlpha?: number;
}

export type CropTarget = NormalizedBoundingBox | NormalizedDetection | RoiKeyframe;

export interface CropPlanRequest {
  /** Width / height of the desired output frame, e.g. 9 / 16. */
  readonly aspectRatio: number;
  /**
   * Source pixel width / height. If omitted, the backwards-compatible default
   * is 1 (square source); callers should provide this for real video.
   */
  readonly sourceAspectRatio?: number | undefined;
  /** Optional source dimensions from which sourceAspectRatio can be derived. */
  readonly sourceWidth?: number | undefined;
  readonly sourceHeight?: number | undefined;
  /** Selected cam (streamer camera overlay) region, if one is available. */
  readonly cam?: CropTarget | undefined;
  /** Selected chat overlay region, if one is available. */
  readonly chat?: CropTarget | undefined;
  /** Selected content (main gameplay/stream canvas) region, if one is available. */
  readonly content?: CropTarget | undefined;
  /** Extra normalized padding around the selected union. Default: 0. */
  readonly padding?: number;
}

export type CropPlanMode =
  | "center"
  | "cam"
  | "chat"
  | "content"
  | "cam_chat"
  | "cam_content"
  | "chat_content"
  | "all";

export interface CropPlan {
  readonly aspectRatio: number;
  readonly sourceAspectRatio: number;
  /** Normalized crop rectangle to pass to a renderer. */
  readonly crop: NormalizedBoundingBox;
  readonly bbox: NormalizedBoundingBox;
  readonly included: Readonly<{ readonly cam: boolean; readonly chat: boolean; readonly content: boolean }>;
  readonly mode: CropPlanMode;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function planCliArgs(options: TrackingPlanOptions): string[] {
  const args = ["plan"];
  const minConfidence = options.minConfidence ?? options.confidenceThreshold;
  const maxGap = options.maxGapSeconds ?? options.maxGap;
  const smoothing = options.smoothingFactor ?? options.smoothingAlpha;
  if (minConfidence !== undefined) args.push("--min-confidence", String(minConfidence));
  if (maxGap !== undefined) args.push("--max-gap", String(maxGap));
  if (smoothing !== undefined) args.push("--smoothing", String(smoothing));
  return args;
}

function planInputJson(input: readonly NormalizedDetection[] | NormalizedTrackingResult): string {
  if (Array.isArray(input)) return JSON.stringify(input);
  if (isRecord(input) && Array.isArray(input.detections)) return JSON.stringify(input);
  throw new TypeError("Tracking plan input must be a detection array or NormalizedTrackingResult.");
}

function throwPlanningError(error: CutEngineError): never {
  const message = error.message;
  if (
    message.includes("must be a finite") ||
    message.includes("must be between") ||
    message.includes("must be greater")
  ) {
    throw new RangeError(message);
  }
  throw new TypeError(message);
}

function throwCropError(error: CutEngineError): never {
  throw new RangeError(error.message);
}

/**
 * Build stable, class-separated tracking segments and smoothed ROI keyframes.
 *
 * Detections are filtered, sorted, grouped by semantic class + track ID, and
 * split when the time gap exceeds `maxGapSeconds`. Smoothing is an exponential
 * moving average over each segment, so a gap never leaks an old ROI into a new
 * segment. The function never mutates its input.
 */
export function buildTrackingPlan(
  input: readonly NormalizedDetection[] | NormalizedTrackingResult,
  options: TrackingPlanOptions = {},
): TrackingPlan {
  try {
    return invokeCutEngineJson<TrackingPlan>(planCliArgs(options), {
      input: planInputJson(input),
    });
  } catch (error) {
    if (error instanceof CutEngineError) throwPlanningError(error);
    throw error;
  }
}

export const planTracking = buildTrackingPlan;

/**
 * Fit a target-aspect crop around selected cam, chat, and/or content regions.
 *
 * The union is expanded to the requested aspect ratio, centered on the
 * selected regions, then shifted into the normalized frame. If the requested
 * aspect ratio cannot contain a target at once, the largest valid crop is
 * returned and `included` reports the loss; missing targets use a centered
 * aspect-ratio crop as a deterministic fallback.
 */
export function planCropForAspectRatio(request: CropPlanRequest): CropPlan {
  try {
    return invokeCutEngineJson<CropPlan>(["crop"], { input: JSON.stringify(request) });
  } catch (error) {
    if (error instanceof CutEngineError) throwCropError(error);
    throw error;
  }
}

export const buildCropPlan = planCropForAspectRatio;

export { VISION_TARGETS };
