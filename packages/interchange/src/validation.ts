import { extname } from "node:path";
import {
  DEFAULT_PYTHON_EXECUTABLE,
  DEFAULT_TIMEOUT_MS,
  OTIO_NATIVE_ADAPTERS,
  type ExportOptions,
  type ImportOptions,
  type InterchangeConfig,
  type InterchangeIssue,
  type InterchangeReport,
} from "./types.js";
import {
  InterchangeConfigError,
  InterchangeFormatError,
  InterchangePathError,
} from "./errors.js";

const ADAPTER_NAME_RE = /^[A-Za-z0-9][A-Za-z0-9_.-]{0,127}$/;

function nonEmptyText(value: unknown, field: string, ErrorType: typeof InterchangeConfigError): string {
  if (typeof value !== "string" || value.trim().length === 0) {
    throw new ErrorType(`${field} must be a non-empty string.`, field);
  }
  if (value.includes("\0")) {
    throw new ErrorType(`${field} must not contain a NUL byte.`, field);
  }
  return value.trim();
}

export function validatePath(value: unknown, field: string): string {
  return nonEmptyText(value, field, InterchangePathError);
}

export function validateAdapterName(value: unknown, field = "adapter"): string {
  const adapter = nonEmptyText(value, field, InterchangeFormatError);
  if (!ADAPTER_NAME_RE.test(adapter)) {
    throw new InterchangeFormatError(
      `${field} must contain only letters, numbers, dots, underscores, and hyphens (max 128 characters).`,
      field,
    );
  }
  return adapter;
}

export function isNativeAdapter(value: string): value is (typeof OTIO_NATIVE_ADAPTERS)[number] {
  return (OTIO_NATIVE_ADAPTERS as readonly string[]).includes(value);
}

/** Normalize friendly format spellings without asserting optional plugins exist. */
export function normalizeAdapter(value: unknown, outputPath?: string): string {
  if (value !== undefined) {
    const format = validateAdapterName(value, "format");
    if (format === "otio" || format === "json" || format === "otio-json") return "otio_json";
    if (format === "otioz" || format === "otio-z") return "otioz";
    return format;
  }

  if (outputPath !== undefined) {
    const extension = extname(outputPath).toLocaleLowerCase("en-US");
    if (extension === ".otioz") return "otioz";
    if (extension === ".otio" || extension === ".json") return "otio_json";
  }

  // The native JSON adapter is the safest deterministic default. OTIO still
  // receives an explicit adapter on export; no optional format is claimed.
  return "otio_json";
}

export interface ValidatedConfig {
  readonly pythonExecutable: string;
  readonly bridgeScript?: string;
  readonly cwd?: string;
  readonly timeoutMs: number;
  readonly env?: NodeJS.ProcessEnv;
}

function positiveInteger(value: unknown, field: string): number {
  if (typeof value !== "number" || !Number.isFinite(value) || !Number.isInteger(value) || value <= 0) {
    throw new InterchangeConfigError(`${field} must be a positive integer.`, field);
  }
  return value;
}

export function validateConfig(input: InterchangeConfig = {}): ValidatedConfig {
  if (typeof input !== "object" || input === null || Array.isArray(input)) {
    throw new InterchangeConfigError("Configuration must be an object.");
  }

  const pythonExecutable =
    input.pythonExecutable === undefined
      ? DEFAULT_PYTHON_EXECUTABLE
      : nonEmptyText(input.pythonExecutable, "pythonExecutable", InterchangeConfigError);
  const bridgeScript =
    input.bridgeScript === undefined
      ? undefined
      : validatePath(input.bridgeScript, "bridgeScript");
  const cwd = input.cwd === undefined ? undefined : validatePath(input.cwd, "cwd");
  const timeoutMs = input.timeoutMs === undefined ? DEFAULT_TIMEOUT_MS : positiveInteger(input.timeoutMs, "timeoutMs");
  const result: ValidatedConfig = { pythonExecutable, timeoutMs };
  if (bridgeScript !== undefined) Object.assign(result, { bridgeScript });
  if (cwd !== undefined) Object.assign(result, { cwd });
  if (input.env !== undefined) Object.assign(result, { env: { ...input.env } });
  return result;
}

export const validateInterchangeConfig = validateConfig;
export const validateOtioConfig = validateConfig;

export function validateImportOptions(options: ImportOptions): ImportOptions {
  if (options === undefined || options === null || typeof options !== "object" || Array.isArray(options)) {
    throw new InterchangeConfigError("Import options must be an object.");
  }
  if (options.adapter !== undefined) validateAdapterName(options.adapter);
  if (options.timeoutMs !== undefined) positiveInteger(options.timeoutMs, "timeoutMs");
  return options;
}

export function validateExportOptions(options: ExportOptions): ExportOptions {
  if (options === undefined || options === null || typeof options !== "object" || Array.isArray(options)) {
    throw new InterchangeConfigError("Export options must be an object.");
  }
  if (options.adapter !== undefined) validateAdapterName(options.adapter);
  if (options.format !== undefined) validateAdapterName(options.format, "format");
  if (options.adapter !== undefined && options.format !== undefined) {
    const adapter = normalizeAdapter(options.adapter);
    const format = normalizeAdapter(options.format);
    if (adapter !== format) {
      throw new InterchangeFormatError("adapter and format cannot disagree.", "format");
    }
  }
  if (options.timeoutMs !== undefined) positiveInteger(options.timeoutMs, "timeoutMs");
  return options;
}

export function createReport(
  direction: "import" | "export",
  adapter: string | undefined,
  issues: readonly InterchangeIssue[] = [],
): InterchangeReport {
  const deduped: InterchangeIssue[] = [];
  const seen = new Set<string>();
  for (const issue of issues) {
    const key = `${issue.severity}|${issue.code}|${issue.path ?? ""}|${issue.message}`;
    if (seen.has(key)) continue;
    seen.add(key);
    deduped.push(issue);
  }
  const warnings = deduped.filter((issue) => issue.severity === "warning");
  const losses = deduped.filter((issue) => issue.severity === "loss");
  const report: InterchangeReport = {
    direction,
    issues: deduped,
    warnings,
    losses,
    omissions: losses,
  };
  if (adapter !== undefined) Object.assign(report, { adapter });
  return report;
}

export function issue(
  code: string,
  message: string,
  severity: "warning" | "loss",
  path?: string,
  details?: Readonly<Record<string, unknown>>,
): InterchangeIssue {
  const value: InterchangeIssue = { code, message, severity };
  if (path !== undefined) Object.assign(value, { path });
  if (details !== undefined) Object.assign(value, { details });
  return value;
}
