import { fileURLToPath } from "node:url";
import { ProjectSchema, parseProject, type Project } from "@cut/timeline";
import {
  InterchangeAbortError,
  InterchangeConfigError,
  InterchangeOutputError,
  InterchangeProcessError,
  InterchangeTimeoutError,
} from "./errors.js";
import { createNodeProcessRunner, invokeProcessRunner, linkAbortSignal } from "./process.js";
import {
  inspectProjectLosses,
  mergeReports,
  normalizeBridgeReport,
} from "./report.js";
import {
  normalizeAdapter,
  validateAdapterName,
  validateConfig,
  validateExportOptions,
  validateImportOptions,
  validatePath,
  isNativeAdapter,
  createReport,
  type ValidatedConfig,
} from "./validation.js";
import type {
  ExportOptions,
  ExportResult,
  ImportOptions,
  ImportResult,
  InterchangeConfig,
  InterchangeDependencies,
  InterchangeReport,
  ProcessInvocation,
  ProcessResult,
  ProcessRunnerLike,
} from "./types.js";

export const DEFAULT_BRIDGE_SCRIPT = fileURLToPath(new URL("../python/otio_bridge.py", import.meta.url));

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
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

function formatZodError(error: unknown): string {
  if (isRecord(error) && Array.isArray(error.issues)) {
    const first = error.issues[0];
    if (isRecord(first) && typeof first.message === "string") {
      const path = Array.isArray(first.path) ? first.path.join(".") : "project";
      return `${path}: ${first.message}`;
    }
  }
  return error instanceof Error ? error.message : String(error);
}

function bridgeErrorMessage(value: unknown): string | undefined {
  if (!isRecord(value)) return undefined;
  if (typeof value.error === "string") return value.error;
  if (isRecord(value.error) && typeof value.error.message === "string") return value.error.message;
  return undefined;
}

function reportForAdapter(direction: "import" | "export", adapter: string | undefined): InterchangeReport {
  if (adapter === undefined || isNativeAdapter(adapter)) return createReport(direction, adapter);
  return createReport(direction, adapter, [
    {
      code: "external_adapter_required",
      message:
        `OTIO adapter ${adapter} is not bundled with cut. It must be installed in the selected Python ` +
        "environment; no FCPXML/AAF support is claimed by this package.",
      severity: "warning",
      details: { adapter },
    },
  ]);
}

interface ParsedBridgeResponse {
  readonly projectValue: unknown;
  readonly reportValue: unknown;
}

function parseBridgeResponse(stdout: string, direction: "import" | "export"): ParsedBridgeResponse {
  const trimmed = stdout.trim();
  if (trimmed.length === 0) {
    throw new InterchangeOutputError("OTIO bridge returned no JSON output.");
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(trimmed) as unknown;
  } catch (error) {
    throw new InterchangeOutputError(
      `OTIO bridge returned invalid JSON: ${error instanceof Error ? error.message : String(error)}`,
      undefined,
      error,
    );
  }

  if (!isRecord(parsed)) {
    throw new InterchangeOutputError("OTIO bridge response must be a JSON object.");
  }
  if (parsed.ok === false || parsed.success === false) {
    throw new InterchangeOutputError(bridgeErrorMessage(parsed) ?? "OTIO bridge reported a conversion failure.");
  }

  if (direction === "export") {
    return {
      projectValue: undefined,
      reportValue: parsed.report ?? parsed,
    };
  }

  // The bundled bridge returns { ok, project, report }. Accepting a direct
  // v2 project as well keeps the process boundary convenient for test fakes.
  let projectValue: unknown = parsed.project;
  if (projectValue === undefined && isRecord(parsed.result)) {
    projectValue = parsed.result.project ?? parsed.result.timeline;
  }
  if (projectValue === undefined && isRecord(parsed.timeline) && parsed.timeline.version === 2) {
    projectValue = parsed.timeline;
  }
  if (projectValue === undefined && parsed.version === 2) projectValue = parsed;
  if (projectValue === undefined) {
    throw new InterchangeOutputError("OTIO bridge response does not contain a v2 project.");
  }
  return { projectValue, reportValue: parsed.report ?? parsed };
}

async function invokeWithControls(options: {
  readonly runner: ProcessRunnerLike;
  readonly invocation: ProcessInvocation;
  readonly signal: AbortSignal | undefined;
  readonly timeoutMs: number;
}): Promise<ProcessResult> {
  const { controller, dispose } = linkAbortSignal(options.signal);
  let timer: NodeJS.Timeout | undefined;
  let settled = false;
  let timedOut = false;
  let abortRequested = controller.signal.aborted;
  if (abortRequested) {
    dispose();
    throw new InterchangeAbortError();
  }

  const invocation: ProcessInvocation = { ...options.invocation, signal: controller.signal, timeoutMs: options.timeoutMs };
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
        complete(() => reject(new InterchangeAbortError()));
      };
      options.signal?.addEventListener("abort", onAbort, { once: true });
      removeAbortListener = (): void => options.signal?.removeEventListener("abort", onAbort);

      timer = setTimeout(() => {
        timedOut = true;
        controller.abort();
        complete(() => reject(new InterchangeTimeoutError(options.timeoutMs)));
      }, options.timeoutMs);
      timer.unref?.();

      promise
        .then(
          (value) => complete(() => resolve(value)),
          (error: unknown) => complete(() => reject(error)),
        )
        .finally(() => removeAbortListener());
    });
    return result;
  } catch (error) {
    if (timedOut) throw new InterchangeTimeoutError(options.timeoutMs);
    if (abortRequested || options.signal?.aborted || abortLike(error)) throw new InterchangeAbortError();
    throw error;
  } finally {
    dispose();
  }
}

function withOptional<T extends object, K extends keyof T>(target: T, key: K, value: T[K] | undefined): void {
  if (value !== undefined) target[key] = value;
}

/**
 * External OpenTimelineIO interchange client. The constructor does no
 * filesystem or Python dependency probing; conversion occurs only on call.
 */
export class OpenTimelineIOAdapter {
  readonly config: ValidatedConfig;
  readonly bridgeScript: string;
  private readonly runner: ProcessRunnerLike;

  constructor(config: InterchangeConfig = {}, dependencies: InterchangeDependencies = {}) {
    this.config = validateConfig(config);
    this.bridgeScript = this.config.bridgeScript ?? DEFAULT_BRIDGE_SCRIPT;
    this.runner = dependencies.runner ?? dependencies.processRunner ?? createNodeProcessRunner();
  }

  /** Build the exact Python invocation without starting a process. */
  buildImportInvocation(inputPath: string, options: ImportOptions = {}): ProcessInvocation {
    const path = validatePath(inputPath, "inputPath");
    validateImportOptions(options);
    const args = [this.bridgeScript, "import", "--input", path];
    if (options.adapter !== undefined) args.push("--adapter", validateAdapterName(options.adapter));
    const invocation: ProcessInvocation = {
      command: this.config.pythonExecutable,
      args,
      timeoutMs: options.timeoutMs ?? this.config.timeoutMs,
    };
    withOptional(invocation, "cwd", this.config.cwd);
    withOptional(invocation, "env", this.config.env);
    return invocation;
  }

  /** Build the exact Python invocation and JSON request body without running it. */
  buildExportInvocation(projectValue: Project | unknown, outputPath: string, options: ExportOptions = {}): ProcessInvocation {
    const project = this.validateProject(projectValue);
    const path = validatePath(outputPath, "outputPath");
    validateExportOptions(options);
    const adapterInput = options.adapter ?? options.format;
    const adapter = normalizeAdapter(adapterInput, path);
    const args = [this.bridgeScript, "export", "--output", path, "--adapter", adapter];
    const invocation: ProcessInvocation = {
      command: this.config.pythonExecutable,
      args,
      stdin: JSON.stringify({ project }),
      timeoutMs: options.timeoutMs ?? this.config.timeoutMs,
    };
    withOptional(invocation, "cwd", this.config.cwd);
    withOptional(invocation, "env", this.config.env);
    return invocation;
  }

  async import(inputPath: string, options: ImportOptions = {}): Promise<ImportResult> {
    const path = validatePath(inputPath, "inputPath");
    const invocation = this.buildImportInvocation(path, options);
    const adapter = options.adapter === undefined ? undefined : validateAdapterName(options.adapter);
    let processResult: ProcessResult;
    try {
      processResult = await invokeWithControls({
        runner: this.runner,
        invocation,
        signal: options.signal,
        timeoutMs: options.timeoutMs ?? this.config.timeoutMs,
      });
    } catch (error) {
      if (error instanceof InterchangeAbortError || error instanceof InterchangeTimeoutError) throw error;
      if (error instanceof InterchangeProcessError) throw error;
      throw new InterchangeProcessError(
        `OTIO bridge could not be started: ${error instanceof Error ? error.message : String(error)}`,
        { invocation, cause: error },
      );
    }

    const exitCode = resultExitCode(processResult);
    const stderr = resultText(processResult, "stderr");
    if (exitCode !== 0) {
      throw new InterchangeProcessError(
        `OTIO bridge exited with code ${String(exitCode)}${stderr.trim() ? `: ${stderr.trim()}` : "."}`,
        { exitCode, stderr, invocation },
      );
    }

    const parsed = parseBridgeResponse(resultText(processResult, "stdout"), "import");
    let project: Project;
    try {
      project = ProjectSchema.parse(parsed.projectValue);
    } catch (error) {
      throw new InterchangeOutputError(`OTIO bridge returned an invalid cut v2 project: ${formatZodError(error)}`, undefined, error);
    }

    const bridgeReport = normalizeBridgeReport(parsed.reportValue, "import", adapter);
    const featureReport = inspectProjectLosses(project, "import");
    const adapterReport = reportForAdapter("import", adapter);
    return {
      project,
      inputPath: path,
      report: mergeReports("import", adapter, bridgeReport, featureReport, adapterReport),
      ...(adapter === undefined ? {} : { adapter }),
    };
  }

  async export(projectValue: Project | unknown, outputPath: string, options: ExportOptions = {}): Promise<ExportResult> {
    const project = this.validateProject(projectValue);
    const path = validatePath(outputPath, "outputPath");
    const invocation = this.buildExportInvocation(project, path, options);
    const adapter = normalizeAdapter(options.adapter ?? options.format, path);
    let processResult: ProcessResult;
    try {
      processResult = await invokeWithControls({
        runner: this.runner,
        invocation,
        signal: options.signal,
        timeoutMs: options.timeoutMs ?? this.config.timeoutMs,
      });
    } catch (error) {
      if (error instanceof InterchangeAbortError || error instanceof InterchangeTimeoutError) throw error;
      if (error instanceof InterchangeProcessError) throw error;
      throw new InterchangeProcessError(
        `OTIO bridge could not be started: ${error instanceof Error ? error.message : String(error)}`,
        { invocation, cause: error },
      );
    }

    const exitCode = resultExitCode(processResult);
    const stderr = resultText(processResult, "stderr");
    if (exitCode !== 0) {
      throw new InterchangeProcessError(
        `OTIO bridge exited with code ${String(exitCode)}${stderr.trim() ? `: ${stderr.trim()}` : "."}`,
        { exitCode, stderr, invocation },
      );
    }
    const parsed = parseBridgeResponse(resultText(processResult, "stdout"), "export");
    const bridgeReport = normalizeBridgeReport(parsed.reportValue, "export", adapter);
    const featureReport = inspectProjectLosses(project, "export");
    const adapterReport = reportForAdapter("export", adapter);
    return {
      outputPath: path,
      adapter,
      report: mergeReports("export", adapter, bridgeReport, featureReport, adapterReport),
    };
  }

  async importTimeline(inputPath: string, options: ImportOptions = {}): Promise<ImportResult> {
    return this.import(inputPath, options);
  }

  async exportProject(projectValue: Project | unknown, outputPath: string, options: ExportOptions = {}): Promise<ExportResult> {
    return this.export(projectValue, outputPath, options);
  }

  private validateProject(value: Project | unknown): Project {
    try {
      return parseProject(value);
    } catch (error) {
      throw new InterchangeConfigError(`project must be a valid cut v2 project: ${formatZodError(error)}`);
    }
  }
}

export function createOpenTimelineIOAdapter(
  config: InterchangeConfig = {},
  dependencies: InterchangeDependencies = {},
): OpenTimelineIOAdapter {
  return new OpenTimelineIOAdapter(config, dependencies);
}

export const OtioAdapter = OpenTimelineIOAdapter;
export const OpenTimelineInterchange = OpenTimelineIOAdapter;
export const OtioInterchangeAdapter = OpenTimelineIOAdapter;
export const createOtioAdapter = createOpenTimelineIOAdapter;

export async function importOTIO(
  inputPath: string,
  options: ImportOptions = {},
  dependencies: InterchangeDependencies = {},
  config: InterchangeConfig = {},
): Promise<ImportResult> {
  return new OpenTimelineIOAdapter(config, dependencies).import(inputPath, options);
}

export async function exportOTIO(
  project: Project | unknown,
  outputPath: string,
  options: ExportOptions = {},
  dependencies: InterchangeDependencies = {},
  config: InterchangeConfig = {},
): Promise<ExportResult> {
  return new OpenTimelineIOAdapter(config, dependencies).export(project, outputPath, options);
}

export const importOtio = importOTIO;
export const exportOtio = exportOTIO;
export const importOTIOProject = importOTIO;
export const exportOTIOProject = exportOTIO;
export const importOpenTimelineIO = importOTIO;
export const exportOpenTimelineIO = exportOTIO;
export const importOtioTimeline = importOTIO;
export const exportOtioProject = exportOTIO;
