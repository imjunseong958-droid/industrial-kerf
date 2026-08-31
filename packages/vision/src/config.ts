import {
  DEFAULT_CONFIDENCE,
  DEFAULT_IOU,
  DEFAULT_PYTHON_EXECUTABLE,
  DEFAULT_TIMEOUT_MS,
  DEFAULT_TRACKER,
  VISION_TARGETS,
  YOLO26_SMALL_MODEL,
} from "./types.js";
import type { VisionTarget, Yolo26TrackingConfig } from "./types.js";
import { VisionConfigError } from "./errors.js";

export interface ValidatedYolo26Config {
  readonly weights: string;
  readonly labelMap: ReadonlyMap<string, VisionTarget>;
  readonly pythonExecutable: string;
  readonly runnerScript?: string;
  readonly cwd?: string;
  readonly tracker: string;
  readonly confidence: number;
  readonly iou: number;
  readonly fps?: number;
  readonly device?: string;
  readonly timeoutMs: number;
  readonly unknownLabelPolicy: "ignore" | "error";
}

const STOCK_SMALL_MODEL_RE = /^yolo26s(?:-(?:seg|sem|depth|cls|pose|obb))?\.(?:pt|onnx)$/i;
const REMOTE_WEIGHTS_RE = /^(?:https?|ftp):\/\//i;

const ALLOWED_LABEL_KEYS = new Set(["cam", "chat", "content", "0", "1", "2"]);

const EXPECTED_TARGET_BY_KEY: Readonly<Record<string, VisionTarget>> = {
  cam: "cam",
  chat: "chat",
  content: "content",
  "0": "cam",
  "1": "chat",
  "2": "content",
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function finiteNumber(value: unknown, field: string): number {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    throw new VisionConfigError(`${field} must be a finite number.`, field);
  }
  return value;
}

function threshold(value: unknown, field: string, fallback: number): number {
  if (value === undefined) return fallback;
  const number = finiteNumber(value, field);
  if (number < 0 || number > 1) {
    throw new VisionConfigError(`${field} must be between 0 and 1.`, field);
  }
  return number;
}

function positiveNumber(value: unknown, field: string): number {
  const number = finiteNumber(value, field);
  if (number <= 0) {
    throw new VisionConfigError(`${field} must be greater than 0.`, field);
  }
  return number;
}

function positiveInteger(value: unknown, field: string): number {
  const number = positiveNumber(value, field);
  if (!Number.isInteger(number)) {
    throw new VisionConfigError(`${field} must be an integer.`, field);
  }
  return number;
}

function nonEmptyText(value: unknown, field: string): string {
  if (typeof value !== "string" || value.trim().length === 0) {
    throw new VisionConfigError(`${field} must be a non-empty string.`, field);
  }
  if (value.includes("\0")) {
    throw new VisionConfigError(`${field} must not contain a NUL byte.`, field);
  }
  return value.trim();
}

function normalizeLabelKey(value: string): string {
  return value.trim().toLocaleLowerCase("en-US");
}

function isTarget(value: unknown): value is VisionTarget {
  return typeof value === "string" && (VISION_TARGETS as readonly string[]).includes(value);
}

function assertTargets(value: unknown): void {
  if (value === undefined) return;
  if (!Array.isArray(value) || value.length !== VISION_TARGETS.length) {
    throw new VisionConfigError(
      `targets must contain exactly ${VISION_TARGETS.join(" and ")}.`,
      "targets",
    );
  }
  const values = new Set(value);
  if (values.size !== VISION_TARGETS.length || !VISION_TARGETS.every((target) => values.has(target))) {
    throw new VisionConfigError(
      `targets must contain exactly ${VISION_TARGETS.join(" and ")}.`,
      "targets",
    );
  }
}

function assertCustomWeights(weights: string): void {
  if (REMOTE_WEIGHTS_RE.test(weights)) {
    throw new VisionConfigError(
      "weights must be a local custom-trained checkpoint; remote URLs are rejected to prevent implicit downloads.",
      "weights",
    );
  }

  // A bare official identifier makes Ultralytics download the stock COCO
  // checkpoint and cannot satisfy the cam/chat/content contract.
  if (STOCK_SMALL_MODEL_RE.test(weights) || weights === YOLO26_SMALL_MODEL) {
    throw new VisionConfigError(
      `The stock ${YOLO26_SMALL_MODEL} checkpoint is COCO-trained and does not define cam/chat/content. ` +
        "Pass a local custom-trained weights path instead.",
      "weights",
    );
  }
}

/** Identity map for models that emit cam/chat/content and class IDs 0/1/2. */
export function identityLabelMap(): ReadonlyMap<string, VisionTarget> {
  return new Map<string, VisionTarget>([
    ["cam", "cam"],
    ["chat", "chat"],
    ["content", "content"],
    ["0", "cam"],
    ["1", "chat"],
    ["2", "content"],
  ]);
}

function resolveLabelMap(input: Readonly<Record<string, VisionTarget>> | undefined): ReadonlyMap<string, VisionTarget> {
  if (input === undefined || Object.keys(input).length === 0) {
    return identityLabelMap();
  }

  const userMap = new Map<string, VisionTarget>();
  const coveredTargets = new Set<VisionTarget>();

  for (const [rawKey, rawTarget] of Object.entries(input)) {
    const key = normalizeLabelKey(rawKey);
    if (key.length === 0) {
      throw new VisionConfigError("labelMap keys must be non-empty strings.", "labelMap");
    }
    if (!ALLOWED_LABEL_KEYS.has(key)) {
      throw new VisionConfigError(
        `labelMap key "${rawKey}" is not supported. Keys must be cam, chat, content or class IDs 0, 1, 2.`,
        "labelMap",
      );
    }
    if (userMap.has(key)) {
      throw new VisionConfigError(`labelMap contains duplicate key after normalization: ${rawKey}.`, "labelMap");
    }
    const expected = EXPECTED_TARGET_BY_KEY[key];
    if (!isTarget(rawTarget) || rawTarget !== expected) {
      throw new VisionConfigError(`labelMap.${rawKey} must be ${expected}.`, "labelMap");
    }
    userMap.set(key, rawTarget);
    coveredTargets.add(rawTarget);
  }

  if (!VISION_TARGETS.every((target) => coveredTargets.has(target))) {
    throw new VisionConfigError(
      "labelMap must include entries for all three targets (cam, chat, and content).",
      "labelMap",
    );
  }

  const merged = new Map(identityLabelMap());
  for (const [key, target] of userMap) {
    merged.set(key, target);
  }
  return merged;
}

/** Validate and normalize configuration without touching the filesystem. */
export function validateYolo26Config(input: Yolo26TrackingConfig): ValidatedYolo26Config {
  if (!isRecord(input)) {
    throw new VisionConfigError("Configuration must be an object.");
  }

  const weights = nonEmptyText(input.weights, "weights");
  assertCustomWeights(weights);
  assertTargets(input.targets);

  if (input.labelMap !== undefined && !isRecord(input.labelMap)) {
    throw new VisionConfigError("labelMap must be an object.", "labelMap");
  }

  const labelMap = resolveLabelMap(input.labelMap);

  const confidenceValue = input.confidence ?? input.conf;
  if (input.confidence !== undefined && input.conf !== undefined && input.confidence !== input.conf) {
    throw new VisionConfigError("confidence and conf cannot disagree.", "confidence");
  }

  const pythonExecutable =
    input.pythonExecutable === undefined
      ? DEFAULT_PYTHON_EXECUTABLE
      : nonEmptyText(input.pythonExecutable, "pythonExecutable");
  const tracker = input.tracker === undefined ? DEFAULT_TRACKER : nonEmptyText(input.tracker, "tracker");
  const runnerScript =
    input.runnerScript === undefined ? undefined : nonEmptyText(input.runnerScript, "runnerScript");
  const cwd = input.cwd === undefined ? undefined : nonEmptyText(input.cwd, "cwd");
  const device = input.device === undefined ? undefined : nonEmptyText(input.device, "device");
  const confidence = threshold(confidenceValue, "confidence", DEFAULT_CONFIDENCE);
  const iou = threshold(input.iou, "iou", DEFAULT_IOU);
  const fps = input.fps === undefined ? undefined : positiveNumber(input.fps, "fps");
  const timeoutMs = input.timeoutMs === undefined ? DEFAULT_TIMEOUT_MS : positiveInteger(input.timeoutMs, "timeoutMs");
  const unknownLabelPolicy = input.unknownLabelPolicy ?? "ignore";
  if (unknownLabelPolicy !== "ignore" && unknownLabelPolicy !== "error") {
    throw new VisionConfigError("unknownLabelPolicy must be ignore or error.", "unknownLabelPolicy");
  }

  const result: ValidatedYolo26Config = {
    weights,
    labelMap,
    pythonExecutable,
    tracker,
    confidence,
    iou,
    timeoutMs,
    unknownLabelPolicy,
  };
  if (runnerScript !== undefined) Object.assign(result, { runnerScript });
  if (cwd !== undefined) Object.assign(result, { cwd });
  if (fps !== undefined) Object.assign(result, { fps });
  if (device !== undefined) Object.assign(result, { device });
  return result;
}

export const validateVisionConfig = validateYolo26Config;
