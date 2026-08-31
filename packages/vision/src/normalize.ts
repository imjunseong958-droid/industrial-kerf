import { CutEngineError, invokeCutEngineJson } from "@cut/engine";
import { VisionOutputError } from "./errors.js";
import type { NormalizedDetection, VisionTarget } from "./types.js";

interface ParsedTrackingOutput {
  readonly detections: readonly NormalizedDetection[];
  readonly frameCount: number;
  readonly fps?: number;
}

export function parseAndNormalizeTrackingOutput(
  stdout: string,
  options: {
    readonly labelMap: ReadonlyMap<string, VisionTarget>;
    readonly unknownLabelPolicy: "ignore" | "error";
    readonly fps?: number;
    readonly frameWidth?: number;
    readonly frameHeight?: number;
  },
): ParsedTrackingOutput {
  const labelMap = Object.fromEntries(options.labelMap.entries());
  const args = [
    "normalize",
    "--label-map",
    JSON.stringify(labelMap),
    "--unknown",
    options.unknownLabelPolicy,
  ];
  if (options.fps !== undefined) args.push("--fps", String(options.fps));
  if (options.frameWidth !== undefined) args.push("--frame-width", String(options.frameWidth));
  if (options.frameHeight !== undefined) args.push("--frame-height", String(options.frameHeight));

  try {
    return invokeCutEngineJson<ParsedTrackingOutput>(args, { input: stdout });
  } catch (error) {
    if (error instanceof CutEngineError) {
      throw new VisionOutputError(error.message);
    }
    throw error;
  }
}
