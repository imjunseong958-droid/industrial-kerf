import type { ProcessInvocation } from "./types.js";

export type InterchangeErrorCode =
  | "INTERCHANGE_CONFIG_INVALID"
  | "INTERCHANGE_PATH_INVALID"
  | "INTERCHANGE_FORMAT_INVALID"
  | "INTERCHANGE_PROCESS_FAILED"
  | "INTERCHANGE_TIMEOUT"
  | "INTERCHANGE_ABORTED"
  | "INTERCHANGE_OUTPUT_INVALID";

export class InterchangeError extends Error {
  readonly code: InterchangeErrorCode;

  constructor(code: InterchangeErrorCode, message: string, cause?: unknown) {
    super(message);
    this.name = new.target.name;
    this.code = code;
    if (cause !== undefined) this.cause = cause;
  }
}

export class InterchangeConfigError extends InterchangeError {
  readonly field?: string;

  constructor(message: string, field?: string) {
    super("INTERCHANGE_CONFIG_INVALID", message);
    if (field !== undefined) this.field = field;
  }
}

export class InterchangePathError extends InterchangeError {
  readonly field?: string;

  constructor(message: string, field?: string) {
    super("INTERCHANGE_PATH_INVALID", message);
    if (field !== undefined) this.field = field;
  }
}

export class InterchangeFormatError extends InterchangeError {
  readonly field?: string;

  constructor(message: string, field?: string) {
    super("INTERCHANGE_FORMAT_INVALID", message);
    if (field !== undefined) this.field = field;
  }
}

export class InterchangeProcessError extends InterchangeError {
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
    super("INTERCHANGE_PROCESS_FAILED", message, options.cause);
    this.exitCode = options.exitCode ?? null;
    this.stderr = options.stderr ?? "";
    if (options.invocation !== undefined) this.invocation = options.invocation;
  }
}

export class InterchangeTimeoutError extends InterchangeError {
  readonly timeoutMs: number;

  constructor(timeoutMs: number) {
    super("INTERCHANGE_TIMEOUT", `OpenTimelineIO conversion timed out after ${timeoutMs} ms.`);
    this.timeoutMs = timeoutMs;
  }
}

export class InterchangeAbortError extends InterchangeError {
  constructor(message = "OpenTimelineIO conversion was aborted.") {
    super("INTERCHANGE_ABORTED", message);
  }
}

export class InterchangeOutputError extends InterchangeError {
  readonly line?: number;

  constructor(message: string, line?: number, cause?: unknown) {
    super("INTERCHANGE_OUTPUT_INVALID", message, cause);
    if (line !== undefined) this.line = line;
  }
}
