import type { ProcessInvocation } from "./types.js";

export type VisionErrorCode =
  | "VISION_CONFIG_INVALID"
  | "VISION_PROCESS_FAILED"
  | "VISION_TIMEOUT"
  | "VISION_ABORTED"
  | "VISION_OUTPUT_INVALID";

export class VisionError extends Error {
  readonly code: VisionErrorCode;

  constructor(code: VisionErrorCode, message: string, cause?: unknown) {
    super(message);
    this.name = new.target.name;
    this.code = code;
    if (cause !== undefined) {
      this.cause = cause;
    }
  }
}

export class VisionConfigError extends VisionError {
  readonly field?: string;

  constructor(message: string, field?: string) {
    super("VISION_CONFIG_INVALID", message);
    if (field !== undefined) this.field = field;
  }
}

export class VisionProcessError extends VisionError {
  readonly exitCode: number | null;
  readonly stderr: string;
  readonly invocation?: ProcessInvocation;

  constructor(
    message: string,
    options: {
      readonly exitCode?: number | null;
      readonly stderr?: string;
      readonly invocation?: ProcessInvocation;
      readonly cause?: unknown;
    } = {},
  ) {
    super("VISION_PROCESS_FAILED", message, options.cause);
    this.exitCode = options.exitCode ?? null;
    this.stderr = options.stderr ?? "";
    if (options.invocation !== undefined) this.invocation = options.invocation;
  }
}

export class VisionTimeoutError extends VisionError {
  readonly timeoutMs: number;

  constructor(timeoutMs: number) {
    super("VISION_TIMEOUT", `YOLO26 tracking timed out after ${timeoutMs} ms.`);
    this.timeoutMs = timeoutMs;
  }
}

export class VisionAbortError extends VisionError {
  constructor(message = "YOLO26 tracking was aborted.") {
    super("VISION_ABORTED", message);
  }
}

export class VisionOutputError extends VisionError {
  readonly line?: number;

  constructor(message: string, line?: number, cause?: unknown) {
    super("VISION_OUTPUT_INVALID", message, cause);
    if (line !== undefined) this.line = line;
  }
}
