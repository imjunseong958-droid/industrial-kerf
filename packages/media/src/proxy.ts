/**
 * Project-local proxy generation and cache primitives.
 *
 * Cache entries are addressed by a SHA-256 key over the resolved source
 * identity, the source's file stat, and normalized proxy settings.  Writes
 * happen to a uniquely named sibling temporary file and are promoted with a
 * single rename, so a partially written ffmpeg output is never visible as a
 * cache hit.
 */
import { randomBytes } from "node:crypto";
import { mkdir, rename, rm, stat } from "node:fs/promises";
import path from "node:path";
import { CutEngineError, invokeCutEngineJson, spawnCutEngineJson } from "@cut/engine";

export interface ProxySource {
  path: string;
  id?: string;
  [key: string]: unknown;
}

export interface ProxySourceStat {
  size: number;
  mtimeMs: number;
  /** Optional fields make keys resilient when a host only has partial stat data. */
  ctimeMs?: number;
  ino?: number;
  mode?: number;
}

/** Extensible settings object. Hosts can add codec/filter knobs. */
export interface ProxySettings {
  width?: number;
  height?: number;
  maxWidth?: number;
  maxHeight?: number;
  fps?: number;
  videoCodec?: string;
  audioCodec?: string;
  crf?: number;
  preset?: string;
  format?: string;
  extension?: string;
  pixelFormat?: string;
  duration?: number;
  [key: string]: unknown;
}

export interface ProxyProgress {
  ratio: number;
  phase?: "cache" | "render" | "promote" | string;
  message?: string;
  current?: number;
  total?: number;
}

export type ProxyProgressUpdate = ProxyProgress | number;

export interface ProxyRunnerRequest {
  input: string;
  output: string;
  settings: ProxySettings;
  args: string[];
}

export interface ProxyRunnerContext {
  readonly signal: AbortSignal;
  reportProgress(update: ProxyProgressUpdate): void;
  report(update: ProxyProgressUpdate): void;
}

/** Injected runner for tests and custom hosts. */
export type ProxyRunner = (
  request: ProxyRunnerRequest,
  context: ProxyRunnerContext,
) => void | Promise<void>;

export interface ProxyArtifact {
  key: string;
  path: string;
  sourcePath: string;
  settings: ProxySettings;
  fromCache: boolean;
}

export interface ProxyCacheOptions {
  /** Project root used to derive the default `.cut/cache/proxies` directory. */
  projectDir?: string;
  /** Explicit cache directory. It must be a directory owned by the project. */
  cacheDir?: string;
  runner?: ProxyRunner;
}

export interface ProxyCreateOptions {
  signal?: AbortSignal;
  onProgress?: (progress: ProxyProgress) => void;
  /** Rebuild this exact key even when a valid cache entry exists. */
  force?: boolean;
  runner?: ProxyRunner;
}

function sourcePathOf(source: string | ProxySource): string {
  return path.resolve(typeof source === "string" ? source : source.path);
}

function sourceIdOf(source: string | ProxySource): string | undefined {
  return typeof source === "string" ? undefined : source.id;
}

/**
 * Compute a deterministic cache key from already-known source stat data.
 * `source` may be a path or a source object with a stable id.
 */
export function createProxyCacheKey(
  source: string | ProxySource,
  sourceStat: ProxySourceStat,
  settings: ProxySettings,
): string {
  const args = [
    "proxy-key",
    "--source",
    sourcePathOf(source),
    "--size",
    String(sourceStat.size),
    "--mtime-ms",
    String(sourceStat.mtimeMs),
  ];
  const id = sourceIdOf(source);
  if (id !== undefined) args.push("--id", id);
  return invokeCutEngineJson<{ key: string }>(args, {
    input: JSON.stringify(settings),
  }).key;
}

/** Read source stat and compute its key. Missing sources are reported clearly. */
export async function computeProxyCacheKey(
  source: string | ProxySource,
  settings: ProxySettings,
): Promise<string> {
  const sourcePath = sourcePathOf(source);
  const sourceStat = await stat(sourcePath);
  return createProxyCacheKey(source, {
    size: sourceStat.size,
    mtimeMs: sourceStat.mtimeMs,
    ctimeMs: sourceStat.ctimeMs,
    ino: sourceStat.ino,
    mode: sourceStat.mode,
  }, settings);
}

/**
 * Sync when source stat is known; async otherwise.
 */
export function proxyCacheKey(
  source: string | ProxySource,
  sourceStat: ProxySourceStat,
  settings: ProxySettings,
): string;
export function proxyCacheKey(
  source: string | ProxySource,
  settings: ProxySettings,
): Promise<string>;
export function proxyCacheKey(
  source: string | ProxySource,
  statOrSettings: ProxySourceStat | ProxySettings,
  maybeSettings?: ProxySettings,
): string | Promise<string> {
  if (maybeSettings !== undefined) {
    return createProxyCacheKey(source, statOrSettings as ProxySourceStat, maybeSettings);
  }
  return computeProxyCacheKey(source, statOrSettings as ProxySettings);
}

function extensionOf(settings: ProxySettings): string {
  const requested = settings.extension ?? settings.format ?? "mp4";
  const normalized = String(requested).toLowerCase().replace(/[^a-z0-9]/g, "");
  return normalized.length > 0 ? normalized : "mp4";
}

function normalizedProgress(update: ProxyProgressUpdate): ProxyProgress {
  if (typeof update === "number") {
    return { ratio: Number.isFinite(update) ? Math.min(1, Math.max(0, update)) : 0 };
  }
  const ratio = Number.isFinite(update.ratio)
    ? Math.min(1, Math.max(0, update.ratio))
    : 0;
  const result: ProxyProgress = { ratio };
  if (update.phase !== undefined) result.phase = update.phase;
  if (update.message !== undefined) result.message = update.message;
  if (update.current !== undefined) result.current = update.current;
  if (update.total !== undefined) result.total = update.total;
  return result;
}

function abortError(message = "Proxy generation was cancelled"): Error & { code: string } {
  const error = new Error(message) as Error & { code: string };
  error.name = "AbortError";
  error.code = "ABORT_ERR";
  return error;
}

function throwIfAborted(signal: AbortSignal | undefined): void {
  if (signal?.aborted) throw abortError();
}

function assertFiniteDimension(value: unknown, name: string): void {
  if (typeof value === "number" && !Number.isFinite(value)) {
    throw new Error(`${name} must be a finite positive number`);
  }
}

/** Build spawn-safe ffmpeg arguments for the default proxy runner. */
export function proxyFfmpegArgs(
  input: string,
  output: string,
  settings: ProxySettings = {},
): string[] {
  assertFiniteDimension(settings.width, "width");
  assertFiniteDimension(settings.height, "height");
  assertFiniteDimension(settings.maxWidth, "maxWidth");
  assertFiniteDimension(settings.maxHeight, "maxHeight");
  try {
    return invokeCutEngineJson<string[]>(
      ["proxy-args", "--input", input, "--output", output],
      { input: JSON.stringify(settings) },
    );
  } catch (error) {
    if (error instanceof CutEngineError) throw new Error(error.message);
    throw error;
  }
}

/** Default runner: spawn cut-engine proxy-run and parse stderr progress JSON. */
export const ffmpegProxyRunner: ProxyRunner = async (request, context) => {
  await spawnCutEngineJson<{ ok: boolean }>(
    ["proxy-run", "--input", request.input, "--output", request.output],
    {
      input: JSON.stringify(request.settings),
      signal: context.signal,
      timeoutMs: 3_600_000,
      onProgress: (ratio) => context.reportProgress({ ratio, phase: "render" }),
    },
  );
};

function cachePathForKey(cacheDir: string, key: string, extension: string): string {
  if (!/^[a-f0-9]{64}$/u.test(key)) throw new Error(`Invalid proxy cache key: ${key}`);
  return path.join(cacheDir, `${key}.${extension}`);
}

function isWithin(parent: string, candidate: string): boolean {
  const relative = path.relative(parent, candidate);
  return relative === "" || (relative !== ".." && !relative.startsWith(`..${path.sep}`) && !path.isAbsolute(relative));
}

function randomTempName(key: string, extension: string): string {
  // Keep the temporary marker while retaining a muxer-recognizable suffix.
  // ffmpeg otherwise sees only `.tmp` and cannot infer the output format.
  return `.${key}.${process.pid}.${randomBytes(8).toString("hex")}.tmp.${extension}`;
}

function raceAbort<T>(promise: Promise<T>, signal: AbortSignal | undefined): Promise<T> {
  if (!signal) return promise;
  if (signal.aborted) {
    // The shared operation still needs a rejection observer when a caller
    // cancels before it has had a chance to attach to the in-flight promise.
    void promise.catch(() => undefined);
    return Promise.reject(abortError());
  }
  return new Promise<T>((resolvePromise, rejectPromise) => {
    const onAbort = (): void => {
      signal.removeEventListener("abort", onAbort);
      rejectPromise(abortError());
    };
    signal.addEventListener("abort", onAbort, { once: true });
    promise.then(
      (value) => {
        signal.removeEventListener("abort", onAbort);
        resolvePromise(value);
      },
      (error: unknown) => {
        signal.removeEventListener("abort", onAbort);
        rejectPromise(error);
      },
    );
  });
}

/** A cache rooted at one project; it never deletes outside that root. */
export class ProxyCache {
  readonly projectDir: string;
  readonly cacheDir: string;
  readonly runner: ProxyRunner;
  private readonly inFlight = new Map<string, Promise<ProxyArtifact>>();

  constructor(options?: ProxyCacheOptions | string, runner?: ProxyRunner) {
    const normalized: ProxyCacheOptions = typeof options === "string"
      ? { projectDir: options, ...(runner === undefined ? {} : { runner }) }
      : { ...(options ?? {}), ...(runner === undefined ? {} : { runner }) };
    // When a host supplies only cacheDir (common in tests and ephemeral MCP
    // workspaces), treat that directory as the project boundary. This keeps
    // the containment guard useful without requiring a redundant projectDir.
    const hasExplicitProjectDir = normalized.projectDir !== undefined;
    this.projectDir = path.resolve(normalized.projectDir ?? normalized.cacheDir ?? process.cwd());
    this.cacheDir = normalized.cacheDir === undefined
      ? path.resolve(this.projectDir, ".cut", "cache", "proxies")
      : hasExplicitProjectDir
        ? path.resolve(this.projectDir, normalized.cacheDir)
        : this.projectDir;
    if (!isWithin(this.projectDir, this.cacheDir)) {
      throw new Error(`Proxy cache directory must be inside project directory: ${this.cacheDir}`);
    }
    this.runner = normalized.runner ?? ffmpegProxyRunner;
  }

  async key(source: string | ProxySource, settings: ProxySettings): Promise<string> {
    return computeProxyCacheKey(source, settings);
  }

  async pathFor(source: string | ProxySource, settings: ProxySettings): Promise<string> {
    const key = await this.key(source, settings);
    return cachePathForKey(this.cacheDir, key, extensionOf(settings));
  }

  pathForKey(key: string, settings: ProxySettings = {}): string {
    return cachePathForKey(this.cacheDir, key, extensionOf(settings));
  }

  async has(source: string | ProxySource, settings: ProxySettings): Promise<boolean> {
    const output = await this.pathFor(source, settings);
    try {
      const outputStat = await stat(output);
      return outputStat.isFile();
    } catch {
      return false;
    }
  }

  async get(source: string | ProxySource, settings: ProxySettings): Promise<ProxyArtifact | undefined> {
    const key = await this.key(source, settings);
    return this.getByKey(key, sourcePathOf(source), settings);
  }

  private async getByKey(
    key: string,
    sourcePath: string,
    settings: ProxySettings,
  ): Promise<ProxyArtifact | undefined> {
    const output = cachePathForKey(this.cacheDir, key, extensionOf(settings));
    try {
      const outputStat = await stat(output);
      if (!outputStat.isFile()) return undefined;
    } catch {
      return undefined;
    }
    return { key, path: output, sourcePath, settings: { ...settings }, fromCache: true };
  }

  async getOrCreate(
    source: string | ProxySource,
    settings: ProxySettings = {},
    options: ProxyCreateOptions = {},
  ): Promise<ProxyArtifact> {
    throwIfAborted(options.signal);
    const sourcePath = sourcePathOf(source);
    const key = await this.key(source, settings);
    if (!options.force) {
      const existing = await this.getByKey(key, sourcePath, settings);
      if (existing) {
        options.onProgress?.({ ratio: 1, phase: "cache", message: "Proxy cache hit" });
        return existing;
      }
    }
    const shared = this.inFlight.get(key);
    if (shared) return raceAbort(shared, options.signal);
    const pending = this.createForKey(source, settings, key, options);
    this.inFlight.set(key, pending);
    try {
      return await raceAbort(pending, options.signal);
    } finally {
      if (this.inFlight.get(key) === pending) this.inFlight.delete(key);
    }
  }

  /** Alias for getOrCreate. */
  async create(
    source: string | ProxySource,
    settings: ProxySettings = {},
    options: ProxyCreateOptions = {},
  ): Promise<ProxyArtifact> {
    return this.getOrCreate(source, settings, options);
  }

  /** Remove one exact cache entry; no recursive/broad deletion is exposed. */
  async remove(source: string | ProxySource, settings: ProxySettings): Promise<boolean> {
    const output = await this.pathFor(source, settings);
    if (!isWithin(this.cacheDir, output)) throw new Error("Refusing to remove a path outside the proxy cache");
    try {
      await rm(output, { force: false });
      return true;
    } catch (error) {
      const code = (error as NodeJS.ErrnoException).code;
      if (code === "ENOENT") return false;
      throw error;
    }
  }

  private async createForKey(
    source: string | ProxySource,
    settings: ProxySettings,
    key: string,
    options: ProxyCreateOptions,
  ): Promise<ProxyArtifact> {
    const sourcePath = sourcePathOf(source);
    const output = cachePathForKey(this.cacheDir, key, extensionOf(settings));
    const extension = extensionOf(settings);
    const temp = path.join(this.cacheDir, randomTempName(key, extension));
    if (!isWithin(this.cacheDir, output) || !isWithin(this.cacheDir, temp)) {
      throw new Error("Refusing to write outside the proxy cache");
    }
    throwIfAborted(options.signal);
    const progress = (update: ProxyProgressUpdate): void => {
      const normalized = normalizedProgress(update);
      try {
        options.onProgress?.(normalized);
      } catch {
        /* Progress observers cannot fail proxy creation. */
      }
    };
    progress({ ratio: 0, phase: "render" });
    const runner = options.runner ?? this.runner;
    const request: ProxyRunnerRequest = {
      input: sourcePath,
      output: temp,
      settings: { ...settings },
      // Default runner rebuilds argv inside cut-engine proxy-run. Custom
      // runners that spawn ffmpeg themselves still get a prebuilt argv.
      args: runner === ffmpegProxyRunner ? [] : proxyFfmpegArgs(sourcePath, temp, settings),
    };
    const context: ProxyRunnerContext = {
      signal: options.signal ?? new AbortController().signal,
      reportProgress: progress,
      report: progress,
    };
    try {
      const cacheDirReady = mkdir(this.cacheDir, { recursive: true });
      await Promise.all([
        cacheDirReady,
        cacheDirReady.then(() => runner(request, context)),
      ]);
      throwIfAborted(options.signal);
      const tempStat = await stat(temp);
      if (!tempStat.isFile()) throw new Error("Proxy runner did not produce a file");
      // Rename is atomic on one filesystem. If another creator won the race,
      // keep that valid cache entry and discard only our own temporary file.
      if (options.force) {
        // An explicit force request is the one case where replacing an
        // existing entry is intentional. The replacement itself remains one
        // atomic same-directory rename.
        await rename(temp, output);
      } else {
        try {
          await stat(output);
        } catch {
          await rename(temp, output);
        }
      }
      progress({ ratio: 1, phase: "promote" });
      return { key, path: output, sourcePath, settings: { ...settings }, fromCache: false };
    } finally {
      try {
        await rm(temp, { force: true });
      } catch {
        /* Best effort cleanup of this exact temporary path. */
      }
    }
  }
}

export async function createProxy(
  source: string | ProxySource,
  settings: ProxySettings = {},
  options: ProxyCacheOptions & ProxyCreateOptions = {},
): Promise<ProxyArtifact> {
  const cache = new ProxyCache(options);
  return cache.getOrCreate(source, settings, options);
}

export function createProxyCache(
  options?: ProxyCacheOptions | string,
  runner?: ProxyRunner,
): ProxyCache {
  return new ProxyCache(options, runner);
}

export const createProxyCacheKeyAsync = computeProxyCacheKey;
export const buildProxyFfmpegArgs = proxyFfmpegArgs;
export const proxyKey = proxyCacheKey;
export const ProjectProxyCache = ProxyCache;
