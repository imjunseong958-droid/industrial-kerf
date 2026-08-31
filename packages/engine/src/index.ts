/**
 * The TypeScript boundary for the native `cut-engine` process.
 *
 * The engine is an explicitly built dependency. This module never invokes
 * Cargo, and it never silently changes build profiles. In development the
 * default is `target/debug/cut-engine`; an optimized binary must be selected
 * explicitly with `profile: "release"` or `CUT_ENGINE_PROFILE=release`.
 *
 * Commands write one JSON value to stdout. Long-running commands may write
 * progress records (`{"progress": number}`) to stderr, one JSON record per
 * line. All other stderr is retained for diagnostics and is never parsed as
 * command output.
 */
import { spawn, spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const PACKAGE_DIR = path.dirname(fileURLToPath(import.meta.url));
export const CUT_ENGINE_REPO_ROOT = path.resolve(PACKAGE_DIR, "../../..");
const MAX_STDOUT_BYTES = 32 * 1024 * 1024;
const MAX_STDERR_BYTES = 100_000;

export type CutEngineProfile = "debug" | "release";

export type CutEngineErrorCode =
  | "BINARY_NOT_FOUND"
  | "INVALID_PROFILE"
  | "SPAWN_FAILED"
  | "ENGINE_FAILED"
  | "ENGINE_TIMEOUT"
  | "INVALID_JSON";

export interface CutEngineSelectionOptions {
  /** Absolute path (or a command name available on PATH) for the engine. */
  readonly binary?: string;
  /** Build profile to resolve from the repository when `binary` is omitted. */
  readonly profile?: CutEngineProfile;
}

export interface CutEngineInvocationOptions extends CutEngineSelectionOptions {
  readonly input?: string;
  readonly timeoutMs?: number;
}

export class CutEngineError extends Error {
  readonly code: CutEngineErrorCode;
  readonly status: number | null;
  readonly stdout: string;
  readonly stderr: string;

  constructor(
    message: string,
    options: {
      readonly code?: CutEngineErrorCode;
      readonly status?: number | null;
      readonly stdout?: string;
      readonly stderr?: string;
    } = {},
  ) {
    super(message);
    this.name = "CutEngineError";
    this.code = options.code ?? "ENGINE_FAILED";
    this.status = options.status ?? null;
    this.stdout = options.stdout ?? "";
    this.stderr = options.stderr ?? "";
  }
}

function binaryName(): string {
  return process.platform === "win32" ? "cut-engine.exe" : "cut-engine";
}

function profileFrom(options: CutEngineSelectionOptions): CutEngineProfile {
  if (options.profile !== undefined) {
    if (options.profile === "debug" || options.profile === "release") return options.profile;
    throw new CutEngineError(
      `Invalid cut-engine profile ${JSON.stringify(options.profile)}. Expected "debug" or "release".`,
      { code: "INVALID_PROFILE" },
    );
  }
  const configured = process.env["CUT_ENGINE_PROFILE"]?.trim();
  if (configured === undefined || configured === "") return "debug";
  if (configured === "debug" || configured === "release") return configured;
  throw new CutEngineError(
    `Invalid CUT_ENGINE_PROFILE ${JSON.stringify(configured)}. Expected "debug" or "release".`,
    { code: "INVALID_PROFILE" },
  );
}

function profilePath(profile: CutEngineProfile): string {
  return path.join(CUT_ENGINE_REPO_ROOT, "target", profile, binaryName());
}

function isPathLike(value: string): boolean {
  return path.isAbsolute(value) || value.includes("/") || value.includes("\\");
}

function missingBinaryError(
  profile: CutEngineProfile,
  expected: string,
  explicit?: string,
): CutEngineError {
  const selected = explicit === undefined
    ? `No cut-engine ${profile} binary was found at ${expected}.`
    : `CUT_ENGINE_BIN points to a missing cut-engine binary: ${explicit}`;
  const buildCommand = profile === "release"
    ? "cargo build --release -p cut-engine"
    : "cargo build -p cut-engine";
  return new CutEngineError(
    [
      selected,
      `Build it explicitly with: ${buildCommand}`,
      "Or set CUT_ENGINE_BIN to an existing engine executable.",
      "The TypeScript host does not build the Rust engine automatically.",
    ].join("\n"),
    { code: "BINARY_NOT_FOUND" },
  );
}

/** Resolve an already-built engine binary. This function never runs Cargo. */
export function resolveCutEngineBin(options: CutEngineSelectionOptions = {}): string {
  const configuredBinary = options.binary?.trim() || process.env["CUT_ENGINE_BIN"]?.trim();
  if (configuredBinary !== undefined && configuredBinary !== "") {
    if (isPathLike(configuredBinary) && !existsSync(configuredBinary)) {
      const profile = profileFrom(options);
      throw missingBinaryError(profile, configuredBinary, configuredBinary);
    }
    return configuredBinary;
  }

  const profile = profileFrom(options);
  const expected = profilePath(profile);
  if (existsSync(expected)) return expected;
  throw missingBinaryError(profile, expected);
}

/** Resolve an already-built engine binary or throw a diagnostic error. */
export function requireCutEngineBin(options: CutEngineSelectionOptions = {}): string {
  return resolveCutEngineBin(options);
}

export function cutEngineAvailable(options: CutEngineSelectionOptions = {}): boolean {
  try {
    resolveCutEngineBin(options);
    return true;
  } catch {
    return false;
  }
}

export interface CutEngineRun {
  readonly ok: boolean;
  readonly stdout: string;
  readonly stderr: string;
  readonly status: number | null;
  readonly error?: Error;
}

function failureDetail(stdout: string, stderr: string, error?: Error): string {
  return [error?.message, stderr, stdout]
    .filter((part): part is string => part !== undefined && part.trim().length > 0)
    .join("\n")
    .trim()
    .replace(/^Error:\s+/i, "");
}

export function runCutEngine(
  args: readonly string[],
  options: CutEngineInvocationOptions = {},
): CutEngineRun {
  const bin = requireCutEngineBin(options);
  const result = spawnSync(bin, [...args], {
    encoding: "utf8",
    input: options.input,
    timeout: options.timeoutMs ?? 120_000,
    maxBuffer: MAX_STDOUT_BYTES,
  });
  const stdout = result.stdout ?? "";
  const stderr = result.stderr ?? "";
  const status = result.status;
  return {
    ok: status === 0 && result.error === undefined,
    stdout,
    stderr,
    status,
    ...(result.error === undefined ? {} : { error: result.error }),
  };
}

/** Invoke a command and return its stdout, or throw a structured engine error. */
export function invokeCutEngine(
  args: readonly string[],
  options: CutEngineInvocationOptions = {},
): string {
  const result = runCutEngine(args, options);
  if (!result.ok) {
    const code = result.error?.message.includes("timed out") ? "ENGINE_TIMEOUT" : "ENGINE_FAILED";
    throw new CutEngineError(
      failureDetail(result.stdout, result.stderr, result.error) || `cut-engine ${args[0] ?? ""} failed`,
      { code, status: result.status, stdout: result.stdout, stderr: result.stderr },
    );
  }
  return result.stdout;
}

function parseJson<T>(stdout: string): T {
  try {
    return JSON.parse(stdout) as T;
  } catch (error) {
    throw new CutEngineError(
      `cut-engine returned invalid JSON: ${error instanceof Error ? error.message : String(error)}`,
      { code: "INVALID_JSON", stdout },
    );
  }
}

export function invokeCutEngineJson<T>(
  args: readonly string[],
  options: CutEngineInvocationOptions = {},
): T {
  return parseJson<T>(invokeCutEngine(args, options));
}

export interface SpawnCutEngineOptions extends CutEngineInvocationOptions {
  readonly signal?: AbortSignal;
  /** Receives normalized progress ratios in the inclusive range [0, 1]. */
  readonly onProgress?: (ratio: number) => void;
  /** Receives every non-empty stderr line, including progress records. */
  readonly onStderrLine?: (line: string) => void;
}

function abortError(): Error & { code: string } {
  const error = new Error("cut-engine was cancelled") as Error & { code: string };
  error.name = "AbortError";
  error.code = "ABORT_ERR";
  return error;
}

function parseProgressLine(line: string): number | undefined {
  try {
    const parsed = JSON.parse(line) as { progress?: unknown };
    if (typeof parsed.progress !== "number" || !Number.isFinite(parsed.progress)) return undefined;
    return Math.min(1, Math.max(0, parsed.progress));
  } catch {
    return undefined;
  }
}

/**
 * Spawn a command using the same binary selection and JSON contract as the
 * synchronous invocation helpers. Progress is read only from stderr JSONL.
 */
export function spawnCutEngine(
  args: readonly string[],
  options: SpawnCutEngineOptions = {},
): Promise<string> {
  const bin = requireCutEngineBin(options);
  if (options.signal?.aborted) return Promise.reject(abortError());

  return new Promise((resolve, reject) => {
    let child;
    try {
      child = spawn(bin, [...args], { stdio: ["pipe", "pipe", "pipe"] });
    } catch (error) {
      reject(new CutEngineError(
        `Unable to start cut-engine: ${error instanceof Error ? error.message : String(error)}`,
        { code: "SPAWN_FAILED" },
      ));
      return;
    }

    let settled = false;
    let stdout = "";
    let stderr = "";
    let timeout: NodeJS.Timeout | undefined;
    const settle = (error?: Error): void => {
      if (settled) return;
      settled = true;
      if (timeout !== undefined) clearTimeout(timeout);
      options.signal?.removeEventListener("abort", onAbort);
      if (error !== undefined) reject(error);
      else resolve(stdout);
    };
    const terminate = (): void => {
      if (child.exitCode !== null || child.killed) return;
      child.kill("SIGTERM");
      const timer = setTimeout(() => {
        if (child.exitCode === null && !child.killed) child.kill("SIGKILL");
      }, 500);
      timer.unref();
    };
    const onAbort = (): void => {
      terminate();
      settle(abortError());
    };
    options.signal?.addEventListener("abort", onAbort, { once: true });
    if (options.timeoutMs !== undefined) {
      timeout = setTimeout(() => {
        terminate();
        settle(new CutEngineError(`cut-engine ${args[0] ?? ""} timed out`, {
          code: "ENGINE_TIMEOUT",
          stderr,
        }));
      }, options.timeoutMs);
      timeout.unref?.();
    }

    child.stdin.on("error", () => undefined);
    if (options.input !== undefined) child.stdin.end(options.input);
    else child.stdin.end();
    child.stdout.on("data", (data: Buffer) => {
      stdout += data.toString();
      if (stdout.length > MAX_STDOUT_BYTES) {
        stdout = stdout.slice(-MAX_STDOUT_BYTES / 2);
      }
    });

    let stderrBuffer = "";
    const emitStderrLine = (line: string): void => {
      const trimmed = line.trim();
      if (trimmed.length === 0) return;
      options.onStderrLine?.(trimmed);
      const ratio = parseProgressLine(trimmed);
      if (ratio !== undefined) options.onProgress?.(ratio);
    };
    child.stderr.on("data", (data: Buffer) => {
      const text = data.toString();
      stderr += text;
      if (stderr.length > MAX_STDERR_BYTES) stderr = stderr.slice(-MAX_STDERR_BYTES / 2);
      stderrBuffer += text;
      let newline = stderrBuffer.indexOf("\n");
      while (newline >= 0) {
        emitStderrLine(stderrBuffer.slice(0, newline));
        stderrBuffer = stderrBuffer.slice(newline + 1);
        newline = stderrBuffer.indexOf("\n");
      }
    });
    child.once("error", (error) => {
      settle(new CutEngineError(
        failureDetail(stdout, stderr, error) || `Unable to start cut-engine ${args[0] ?? ""}`,
        { code: "SPAWN_FAILED", stdout, stderr },
      ));
    });
    child.once("close", (code) => {
      if (settled) return;
      emitStderrLine(stderrBuffer);
      if (code === 0) {
        options.onProgress?.(1);
        settle();
      } else {
        settle(new CutEngineError(
          failureDetail(stdout, stderr) || `cut-engine ${args[0] ?? ""} failed`,
          { code: "ENGINE_FAILED", status: code, stdout, stderr },
        ));
      }
    });
  });
}

export async function spawnCutEngineJson<T>(
  args: readonly string[],
  options: SpawnCutEngineOptions = {},
): Promise<T> {
  return parseJson<T>(await spawnCutEngine(args, options));
}
