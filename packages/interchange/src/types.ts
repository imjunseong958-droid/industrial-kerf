import type { Project } from "@cut/timeline";

/** The bundled bridge is an OTIO process client only. */
export const DEFAULT_PYTHON_EXECUTABLE = "python3";
export const DEFAULT_TIMEOUT_MS = 120_000;

/** Adapters shipped by the official OTIO package itself. */
export const OTIO_NATIVE_ADAPTERS = ["otio_json", "otioz"] as const;
export type OtioNativeAdapter = (typeof OTIO_NATIVE_ADAPTERS)[number];

export type InterchangeDirection = "import" | "export";
export type InterchangeSeverity = "warning" | "loss";

/** A machine-readable explanation of a conversion caveat or omission. */
export interface InterchangeIssue {
  readonly code: string;
  readonly message: string;
  readonly severity: InterchangeSeverity;
  readonly path?: string;
  readonly details?: Readonly<Record<string, unknown>>;
}

/** Structured report returned for every successful conversion. */
export interface InterchangeReport {
  readonly direction: InterchangeDirection;
  readonly adapter?: string;
  readonly issues: readonly InterchangeIssue[];
  readonly warnings: readonly InterchangeIssue[];
  readonly losses: readonly InterchangeIssue[];
  /** Alias useful to hosts that call unsupported fields omissions. */
  readonly omissions: readonly InterchangeIssue[];
}

export interface ProcessInvocation {
  readonly command: string;
  readonly args: readonly string[];
  readonly cwd?: string;
  readonly env?: NodeJS.ProcessEnv;
  readonly signal?: AbortSignal;
  readonly timeoutMs?: number;
  /** UTF-8 request body. The import operation does not send one. */
  readonly stdin?: string;
}

export interface ProcessResult {
  readonly stdout: string;
  readonly stderr: string;
  readonly exitCode: number | null;
  readonly signal?: string | null;
}

/** Injectable process boundary. Tests can provide deterministic bridge output. */
export type ProcessRunner = (
  invocation: ProcessInvocation,
) => PromiseLike<ProcessResult> | ProcessResult;

export interface ProcessRunnerObject {
  run(invocation: ProcessInvocation): PromiseLike<ProcessResult> | ProcessResult;
}

export type ProcessRunnerLike = ProcessRunner | ProcessRunnerObject;

export interface InterchangeConfig {
  /** Python executable in an environment where opentimelineio is installed. */
  readonly pythonExecutable?: string;
  /** Override the bundled bridge path, primarily for packaging/testing. */
  readonly bridgeScript?: string;
  readonly cwd?: string;
  readonly timeoutMs?: number;
  readonly env?: NodeJS.ProcessEnv;
}

export interface InterchangeDependencies {
  readonly runner?: ProcessRunnerLike;
  /** Alias for runner, matching other external-process adapters. */
  readonly processRunner?: ProcessRunnerLike;
}

export interface InterchangeControls {
  readonly timeoutMs?: number;
  readonly signal?: AbortSignal;
}

export interface ImportOptions extends InterchangeControls {
  /** OTIO adapter name. Omit to let OTIO infer it from the input path. */
  readonly adapter?: string;
}

export interface ExportOptions extends InterchangeControls {
  /** Explicit OTIO adapter name, e.g. otio_json or an installed plugin. */
  readonly adapter?: string;
  /** Alias for adapter; `otio` resolves to otio_json. */
  readonly format?: string;
}

export interface ImportResult {
  readonly project: Project;
  readonly inputPath: string;
  readonly adapter?: string;
  readonly report: InterchangeReport;
}

export interface ExportResult {
  readonly outputPath: string;
  readonly adapter: string;
  readonly report: InterchangeReport;
}
