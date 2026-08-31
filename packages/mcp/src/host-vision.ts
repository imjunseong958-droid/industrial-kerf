import { existsSync, readFileSync } from "node:fs";
import path from "node:path";
import { homedir } from "node:os";
import {
  Yolo26Tracker,
  buildTrackingPlan,
  planCropForAspectRatio,
  type ProcessRunnerLike,
  type TrackingPlanOptions,
  type Yolo26TrackingConfig,
} from "@cut/vision";
import type { CutMcpHandler, CutMcpVisionHandlers } from "./server.js";
import { resolveMediaSrc } from "./media-path.js";

export interface CutMcpVisionSetup {
  readonly config?: Yolo26TrackingConfig;
  readonly tracker?: { track(request: unknown): Promise<unknown> };
  readonly runner?: ProcessRunnerLike;
  readonly plan?: TrackingPlanOptions;
  readonly crop?: { aspectRatio: number; padding?: number };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isTrackerLike(value: unknown): value is { track(request: unknown): Promise<unknown> } {
  return typeof value === "object" && value !== null && typeof (value as { track?: unknown }).track === "function";
}

function isVisionConfig(value: unknown): value is Yolo26TrackingConfig {
  return typeof value === "object" && value !== null && typeof (value as { weights?: unknown }).weights === "string";
}

function mergeVisionConfig(
  base: Record<string, unknown>,
  override: Record<string, unknown>,
): Record<string, unknown> {
  const merged: Record<string, unknown> = { ...base, ...override };
  if (
    typeof base.labelMap === "object" &&
    base.labelMap !== null &&
    typeof override.labelMap === "object" &&
    override.labelMap !== null
  ) {
    merged.labelMap = {
      ...(base.labelMap as Record<string, unknown>),
      ...(override.labelMap as Record<string, unknown>),
    };
  }
  return merged;
}

function readJsonSync(file: string): Record<string, unknown> | undefined {
  if (!existsSync(file)) return undefined;
  try {
    const parsed = JSON.parse(readFileSync(file, "utf8")) as unknown;
    return typeof parsed === "object" && parsed !== null && !Array.isArray(parsed)
      ? parsed as Record<string, unknown>
      : undefined;
  } catch {
    return undefined;
  }
}

export function configuredVision(dir: string): CutMcpVisionSetup | undefined {
  const files = [
    path.join(homedir(), ".cut", "vision.json"),
    path.join(homedir(), ".cut", "config.json"),
    path.join(path.resolve(dir), ".cut", "vision.json"),
    path.join(path.resolve(dir), ".cut", "config.json"),
  ];
  let merged: Record<string, unknown> = {};
  let found = false;
  for (const file of files) {
    const raw = readJsonSync(file);
    if (!raw) continue;
    const candidate = path.basename(file) === "config.json" ? raw.vision : raw;
    if (!isRecord(candidate)) continue;
    merged = mergeVisionConfig(merged, candidate);
    found = true;
  }
  if (!found || !isVisionConfig(merged)) return undefined;
  return { config: merged as unknown as Yolo26TrackingConfig };
}

export function resolveVisionSetup(
  dir: string,
  vision?: CutMcpVisionSetup | Yolo26TrackingConfig | { track(request: unknown): Promise<unknown> },
): CutMcpVisionSetup | undefined {
  if (vision === undefined) return configuredVision(dir);
  if (isVisionConfig(vision)) return { config: vision };
  if (isTrackerLike(vision)) return { tracker: vision };
  return vision;
}

function makeVisionTracker(setup: CutMcpVisionSetup): { track(request: unknown): Promise<unknown> } | undefined {
  if (setup.tracker) return setup.tracker;
  if (!setup.config) return undefined;
  return new Yolo26Tracker(setup.config, setup.runner ? { runner: setup.runner } : {});
}

export interface VisionHandlerContext {
  dir: string;
  setup?: CutMcpVisionSetup;
}

export function createVisionHandlers(context: VisionHandlerContext): CutMcpVisionHandlers | undefined {
  const setup = context.setup;
  if (!setup) return undefined;
  const tracker = makeVisionTracker(setup);
  if (!tracker) return undefined;

  const track_objects: CutMcpHandler = async (input) => {
    const value = input as Record<string, unknown>;
    const sourceInput = value.source;
    if (typeof sourceInput !== "string" || sourceInput.length === 0) {
      throw new Error("track_objects requires source");
    }
    const request: {
      source: string;
      fps?: number;
      frameWidth?: number;
      frameHeight?: number;
      maxFrames?: number;
    } = { source: resolveMediaSrc(context.dir, sourceInput) };
    if (typeof value.fps === "number") request.fps = value.fps;
    if (typeof value.frameWidth === "number") request.frameWidth = value.frameWidth;
    if (typeof value.frameHeight === "number") request.frameHeight = value.frameHeight;
    if (typeof value.maxFrames === "number") request.maxFrames = value.maxFrames;

    const tracking = await tracker.track(request);
    const plan = buildTrackingPlan(tracking as never, setup.plan);
    const cropInput = isRecord(value.crop) ? value.crop : undefined;
    const cropOptions = cropInput
      ? {
          aspectRatio: Number(cropInput.aspectRatio),
          ...(cropInput.padding !== undefined ? { padding: Number(cropInput.padding) } : {}),
        }
      : setup.crop;
    const sourceAspectRatio =
      typeof value.frameWidth === "number" && typeof value.frameHeight === "number"
        ? value.frameWidth / value.frameHeight
        : undefined;
    const plannedCrop = cropOptions
      ? planCropForAspectRatio({
          aspectRatio: cropOptions.aspectRatio,
          ...(typeof value.frameWidth === "number" && typeof value.frameHeight === "number"
            ? { sourceWidth: value.frameWidth, sourceHeight: value.frameHeight }
            : {}),
          ...(cropOptions.padding !== undefined ? { padding: cropOptions.padding } : {}),
          cam: plan.keyframesByClass.cam[0],
          chat: plan.keyframesByClass.chat[0],
          content: plan.keyframesByClass.content[0],
        })
      : undefined;
    const cropPlan = plannedCrop
      ? {
          ...plannedCrop,
          ...(sourceAspectRatio !== undefined && cropOptions
            ? {
                requestedAspectRatio: cropOptions.aspectRatio,
                sourceAspectRatio,
                normalizedAspectRatio: cropOptions.aspectRatio / sourceAspectRatio,
              }
            : {}),
        }
      : undefined;
    return {
      ...(typeof tracking === "object" && tracking !== null ? tracking : { tracking }),
      plan,
      ...(cropPlan ? { cropPlan } : {}),
    };
  };

  return { track_objects };
}
