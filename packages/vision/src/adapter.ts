import { fileURLToPath } from "node:url";
import { CutEngineError, invokeCutEngineJson } from "@cut/engine";
import {
  VisionAbortError,
  VisionConfigError,
  VisionOutputError,
  VisionProcessError,
  VisionTimeoutError,
} from "./errors.js";
import { validateYolo26Config, type ValidatedYolo26Config } from "./config.js";
import { createNodeProcessRunner, invokeProcessRunner, linkAbortSignal } from "./process.js";
import { parseAndNormalizeTrackingOutput } from "./normalize.js";
import {
  DEFAULT_TIMEOUT_MS,
  VISION_TARGETS,
  type NormalizedTrackingResult,
  type ProcessInvocation,
  type ProcessResult,
  type ProcessRunnerLike,
  type TrackOptions,
  type TrackRequest,
  type Yolo26TrackingConfig,
} from "./types.js";

export const DEFAULT_ULTRALYTICS_RUNNER_SCRIPT = fileURLToPath(
  new URL("../python/ultralytics_track.py", import.meta.url),
);

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function resultExitCode(value: ProcessResult): number | null {
  if (typeof value.exitCode === "number" || value.exitCode === null) return value.exitCode;
  const legacy = (value as unknown as Record<string, unknown>).code;
  return typeof legacy === "number" || legacy === null ? legacy : null;
}

function resultText(value: ProcessResult, key: "stdout" | "stderr"): string {
  const text = value[key];
  return typeof text === "string" ? text : String(text ?? "");
}

function abortLike(error: unknown): boolean {
  if (!isRecord(error)) return false;
  return error.name === "AbortError" || error.code === "ABORT_ERR";
}

function sourceFromRequest(request: string | TrackRequest): TrackRequest {
  if (typeof request === "string") return { source: request };
  return request;
}

function validateRequest(request: TrackRequest): TrackRequest {
  if (!isRecord(request) || typeof request.source !== "string" || request.source.trim().length === 0) {
    throw new VisionConfigError("source must be a non-empty string.", "source");
  }
  if (request.source.includes("\0")) throw new VisionConfigError("source must not contain a NUL byte.", "source");
  for (const [field, value] of [
    ["fps", request.fps],
    ["frameWidth", request.frameWidth],
    ["frameHeight", request.frameHeight],
  ] as const) {
    if (value !== undefined && (typeof value !== "number" || !Number.isFinite(value) || value <= 0)) {
      throw new VisionConfigError(`${field} must be a finite number greater than zero.`, field);
    }
  }
  if (request.maxFrames !== undefined && (!Number.isInteger(request.maxFrames) || request.maxFrames <= 0)) {
    throw new VisionConfigError("maxFrames must be a positive integer.", "maxFrames");
  }
  if (request.timeoutMs !== undefined && (!Number.isInteger(request.timeoutMs) || request.timeoutMs <= 0)) {
    throw new VisionConfigError("timeoutMs must be a positive integer.", "timeoutMs");
  }
  return { ...request, source: request.source.trim() };
}

function buildInvocation(
  config: ValidatedYolo26Config,
  request: TrackRequest,
): ProcessInvocation {
  const script = config.runnerScript ?? DEFAULT_ULTRALYTICS_RUNNER_SCRIPT;
  const fps = request.fps ?? config.fps;
  const timeoutMs = request.timeoutMs ?? config.timeoutMs ?? DEFAULT_TIMEOUT_MS;
  const args = [
    script,
    "--model",
    config.weights,
    "--source",
    request.source,
    "--tracker",
    config.tracker,
    "--conf",
    String(config.confidence),
    "--iou",
    String(config.iou),
  ];
  if (config.device !== undefined) args.push("--device", config.device);
  if (fps !== undefined) args.push("--fps", String(fps));
  if (request.maxFrames !== undefined) args.push("--max-frames", String(request.maxFrames));

  const invocation: ProcessInvocation = {
    command: config.pythonExecutable,
    args,
    timeoutMs,
  };
  if (config.cwd !== undefined) Object.assign(invocation, { cwd: config.cwd });
  return invocation;
}

function isOutputValidationError(message: string): boolean {
  return /must be|track_id|bbox|confidence|detection|frame|Runner output|valid JSON|labelMap|label_map/i.test(message);
}

function mapCutEngineError(error: CutEngineError): never {
  if (isOutputValidationError(error.message)) {
    throw new VisionOutputError(error.message);
  }
  throw new VisionProcessError(error.message, {
    exitCode: error.status,
    stderr: error.stderr,
  });
}

interface InvokeOptions {
  readonly runner: ProcessRunnerLike;
  readonly invocation: ProcessInvocation;
  readonly signal: AbortSignal | undefined;
  readonly timeoutMs: number;
}

async function invokeWithControls(options: InvokeOptions): Promise<ProcessResult> {
  const { controller, dispose } = linkAbortSignal(options.signal);
  let timer: NodeJS.Timeout | undefined;
  let settled = false;
  let timedOut = false;
  let abortRequested = controller.signal.aborted;
  if (abortRequested) {
    dispose();
    throw new VisionAbortError();
  }

  const invocation: ProcessInvocation = { ...options.invocation, signal: controller.signal };
  try {
    const promise = Promise.resolve().then(() => invokeProcessRunner(options.runner, invocation));
    const result = await new Promise<ProcessResult>((resolve, reject) => {
      let removeAbortListener = (): void => undefined;
      const complete = (callback: () => void): void => {
        if (settled) return;
        settled = true;
        if (timer !== undefined) clearTimeout(timer);
        removeAbortListener();
        callback();
      };

      const onAbort = (): void => {
        abortRequested = true;
        controller.abort();
        complete(() => reject(new VisionAbortError()));
      };
      options.signal?.addEventListener("abort", onAbort, { once: true });
      removeAbortListener = (): void => options.signal?.removeEventListener("abort", onAbort);

      timer = setTimeout(() => {
        timedOut = true;
        controller.abort();
        complete(() => reject(new VisionTimeoutError(options.timeoutMs)));
      }, options.timeoutMs);
      timer.unref?.();

      promise.then(
        (value) => complete(() => resolve(value)),
        (error: unknown) => complete(() => reject(error)),
      ).finally(() => removeAbortListener());
    });
    return result;
  } catch (error) {
    if (timedOut) throw new VisionTimeoutError(options.timeoutMs);
    if (abortRequested || options.signal?.aborted || abortLike(error)) throw new VisionAbortError();
    throw error;
  } finally {
    dispose();
  }
}

/** A typed adapter around an externally installed Ultralytics runner. */
export class Yolo26Tracker {
  readonly config: ValidatedYolo26Config;
  private readonly runner: ProcessRunnerLike;
  private readonly usesDefaultRunner: boolean;

  constructor(config: Yolo26TrackingConfig, dependencies: { readonly runner?: ProcessRunnerLike; readonly processRunner?: ProcessRunnerLike } = {}) {
    this.config = validateYolo26Config(config);
    const runner = dependencies.runner ?? dependencies.processRunner;
    this.usesDefaultRunner = runner === undefined;
    this.runner = runner ?? createNodeProcessRunner();
  }

  /** Build the exact Python process call without running it. */
  buildInvocation(request: string | TrackRequest): ProcessInvocation {
    const normalizedRequest = validateRequest(sourceFromRequest(request));
    return buildInvocation(this.config, normalizedRequest);
  }

  async track(request: string | TrackRequest, options: TrackOptions = {}): Promise<NormalizedTrackingResult> {
    const baseRequest = validateRequest(sourceFromRequest(request));
    const merged: TrackRequest = {
      ...baseRequest,
      ...(options.fps === undefined ? {} : { fps: options.fps }),
      ...(options.frameWidth === undefined ? {} : { frameWidth: options.frameWidth }),
      ...(options.frameHeight === undefined ? {} : { frameHeight: options.frameHeight }),
      ...(options.timeoutMs === undefined ? {} : { timeoutMs: options.timeoutMs }),
      ...(options.signal === undefined ? {} : { signal: options.signal }),
      ...(options.maxFrames === undefined ? {} : { maxFrames: options.maxFrames }),
    };
    const normalizedRequest = validateRequest(merged);
    const timeoutMs = normalizedRequest.timeoutMs ?? this.config.timeoutMs;

    if (this.usesDefaultRunner) {
      return this.trackViaCutEngine(normalizedRequest, timeoutMs);
    }

    const invocation = buildInvocation(this.config, normalizedRequest);
    let processResult: ProcessResult;
    try {
      processResult = await invokeWithControls({
        runner: this.runner,
        invocation,
        signal: normalizedRequest.signal,
        timeoutMs,
      });
    } catch (error) {
      if (error instanceof VisionAbortError || error instanceof VisionTimeoutError) throw error;
      if (error instanceof VisionProcessError) throw error;
      throw new VisionProcessError(
        `Ultralytics runner could not be started: ${error instanceof Error ? error.message : String(error)}`,
        { invocation, cause: error },
      );
    }

    const exitCode = resultExitCode(processResult);
    const stderr = resultText(processResult, "stderr");
    if (exitCode !== 0) {
      throw new VisionProcessError(
        `Ultralytics runner exited with code ${String(exitCode)}${stderr.trim() ? `: ${stderr.trim()}` : "."}`,
        { exitCode, stderr, invocation },
      );
    }

    return this.normalizeRunnerOutput(resultText(processResult, "stdout"), normalizedRequest);
  }

  private trackViaCutEngine(request: TrackRequest, timeoutMs: number): NormalizedTrackingResult {
    const labelMap = Object.fromEntries(this.config.labelMap.entries());
    const args = [
      "track",
      "--source",
      request.source,
      "--weights",
      this.config.weights,
      "--label-map",
      JSON.stringify(labelMap),
    ];
    if (this.config.confidence !== undefined) args.push("--conf", String(this.config.confidence));
    if (this.config.iou !== undefined) args.push("--iou", String(this.config.iou));
    const fps = request.fps ?? this.config.fps;
    if (fps !== undefined) args.push("--fps", String(fps));
    if (request.maxFrames !== undefined) args.push("--max-frames", String(request.maxFrames));

    try {
      const rust = invokeCutEngineJson<{
        detections: NormalizedTrackingResult["detections"];
        frameCount: number;
        fps?: number;
      }>(args, { timeoutMs });
      const result: NormalizedTrackingResult = {
        source: request.source,
        model: this.config.weights,
        targets: VISION_TARGETS,
        detections: rust.detections,
        frameCount: rust.frameCount,
      };
      if (rust.fps !== undefined) Object.assign(result, { fps: rust.fps });
      return result;
    } catch (error) {
      if (error instanceof CutEngineError) mapCutEngineError(error);
      throw error;
    }
  }

  private normalizeRunnerOutput(stdout: string, request: TrackRequest): NormalizedTrackingResult {
    const parsed = parseAndNormalizeTrackingOutput(stdout, {
      labelMap: this.config.labelMap,
      unknownLabelPolicy: this.config.unknownLabelPolicy,
      ...(normalizedRequestFps(request, this.config)
        ? { fps: normalizedRequestFps(request, this.config) }
        : {}),
      ...(request.frameWidth === undefined ? {} : { frameWidth: request.frameWidth }),
      ...(request.frameHeight === undefined ? {} : { frameHeight: request.frameHeight }),
    });

    const result: NormalizedTrackingResult = {
      source: request.source,
      model: this.config.weights,
      targets: VISION_TARGETS,
      detections: parsed.detections,
      frameCount: parsed.frameCount,
    };
    if (parsed.fps !== undefined) Object.assign(result, { fps: parsed.fps });
    return result;
  }
}

function normalizedRequestFps(
  request: TrackRequest,
  config: ValidatedYolo26Config,
): number | undefined {
  return request.fps ?? config.fps;
}

export function createYolo26Tracker(
  config: Yolo26TrackingConfig,
  dependencies: { readonly runner?: ProcessRunnerLike; readonly processRunner?: ProcessRunnerLike } = {},
): Yolo26Tracker {
  return new Yolo26Tracker(config, dependencies);
}

export async function trackYolo26(
  config: Yolo26TrackingConfig,
  request: string | TrackRequest,
  dependencies: { readonly runner?: ProcessRunnerLike; readonly processRunner?: ProcessRunnerLike } = {},
  options: TrackOptions = {},
): Promise<NormalizedTrackingResult> {
  return new Yolo26Tracker(config, dependencies).track(request, options);
}

export const runYolo26Tracking = trackYolo26;
