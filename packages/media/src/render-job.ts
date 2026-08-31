/**
 * Background render jobs.
 *
 * The media package intentionally keeps the scheduler independent from
 * ffmpeg.  A job is just a callback that receives an AbortSignal and a
 * structured progress reporter.  This makes the same manager useful for the
 * v1 timeline renderer, the v2 project renderer, and deterministic tests
 * that do not have ffmpeg installed.
 */
import type { Project, Timeline } from "@cut/timeline";
import { render, type RenderOptions } from "./render.js";
import { renderV2, type RenderV2Options } from "./render-v2.js";

export type RenderJobStatus =
  | "queued"
  | "running"
  | "completed"
  | "failed"
  | "cancelled";

export interface RenderProgress {
  /** A clamped completion ratio in the inclusive range [0, 1]. */
  ratio: number;
  /** Optional coarse phase (for example "probe", "render", or "finalize"). */
  phase?: string;
  /** Optional human-readable detail for a host UI. */
  message?: string;
  /** Optional current/total counters when a ratio is not enough context. */
  current?: number;
  total?: number;
}

export type RenderProgressUpdate = RenderProgress | number;

export interface RenderJobError {
  name: string;
  message: string;
  /** Error codes are preserved when the underlying callback provides one. */
  code?: string;
  stack?: string;
  /** A JSON-safe detail value supplied by a callback, when available. */
  details?: unknown;
}

export class RenderJobCancelledError extends Error {
  readonly code = "RENDER_JOB_CANCELLED";

  constructor(message = "Render job was cancelled") {
    super(message);
    this.name = "RenderJobCancelledError";
  }
}

export interface RenderJobContext {
  readonly id: string;
  readonly signal: AbortSignal;
  /** Report either a ratio or a structured progress update. */
  reportProgress(update: RenderProgressUpdate): void;
  /** Short alias useful in injected render callbacks. */
  report(update: RenderProgressUpdate): void;
}

export type RenderJobTask<T = unknown> = (
  context: RenderJobContext,
) => T | Promise<T>;

/** Default executor for hosts that route all render kinds themselves. */
export type RenderJobExecutor = (
  request: RenderJobSpec,
  context: RenderJobContext,
) => unknown | Promise<unknown>;

interface CommonJobSpec<T> {
  /** A caller supplied stable id. It must be unique in this manager. */
  id?: string;
  kind?: string;
  /** Primary callback property. */
  run?: RenderJobTask<T>;
  job?: RenderJobTask<T>;
  execute?: RenderJobTask<T>;
  task?: RenderJobTask<T>;
  callback?: RenderJobTask<T>;
  /** Called for each progress update in addition to manager subscribers. */
  onProgress?: (progress: RenderProgress) => void;
  metadata?: Record<string, unknown>;
}

export interface GenericRenderJobSpec<T = unknown> extends CommonJobSpec<T> {
  kind?: string;
}

export interface RenderV1JobSpec<T = void> extends CommonJobSpec<T> {
  kind: "v1";
  timeline: Timeline;
  options: RenderOptions;
  /** Override the built-in v1 callback for tests or an MCP host. */
  render?: RenderV1Task<T>;
}

export interface RenderV2JobSpec<T = void> extends CommonJobSpec<T> {
  kind: "v2";
  project: Project;
  options: RenderV2Options;
  /** Override the built-in v2 callback for tests or an MCP host. */
  render?: RenderV2Task<T>;
}

export type RenderV1Task<T = void> = (
  timeline: Timeline,
  options: RenderOptions,
  context: RenderJobContext,
) => T | Promise<T>;

export type RenderV2Task<T = void> = (
  project: Project,
  options: RenderV2Options,
  context: RenderJobContext,
) => T | Promise<T>;

export type RenderJobSpec<T = unknown> =
  | GenericRenderJobSpec<T>
  | RenderV1JobSpec<T>
  | RenderV2JobSpec<T>;

export interface RenderJobSnapshot<T = unknown> {
  id: string;
  kind?: string;
  status: RenderJobStatus;
  /** Alias for `status`. */
  state: RenderJobStatus;
  progress: RenderProgress;
  createdAt: number;
  startedAt?: number;
  completedAt?: number;
  result?: T;
  error?: RenderJobError;
  metadata?: Record<string, unknown>;
}

export interface RenderJobManagerOptions {
  /** Maximum number of callbacks running at once. Defaults to 1. */
  concurrency?: number;
  maxConcurrent?: number;
  maxConcurrency?: number;
  /** Default executor used when a job spec has no per-job callback. */
  render?: RenderJobExecutor;
  renderer?: RenderJobExecutor;
  /** Observe every state/progress change. Exceptions are ignored. */
  onUpdate?: (snapshot: RenderJobSnapshot) => void;
}

interface JobRecord {
  id: string;
  kind?: string;
  task: RenderJobTask<unknown>;
  callback?: (progress: RenderProgress) => void;
  metadata?: Record<string, unknown>;
  controller: AbortController;
  status: RenderJobStatus;
  progress: RenderProgress;
  createdAt: number;
  startedAt?: number;
  completedAt?: number;
  value?: unknown;
  error?: RenderJobError;
  cancelRequested: boolean;
  cancelReason?: string;
  resolve: (value: unknown) => void;
  reject: (reason?: unknown) => void;
  promise: Promise<unknown>;
  listeners: Set<(snapshot: RenderJobSnapshot) => void>;
}

function clampRatio(value: number): number {
  if (!Number.isFinite(value)) return 0;
  return Math.min(1, Math.max(0, value));
}

function normalizeProgress(update: RenderProgressUpdate): RenderProgress {
  if (typeof update === "number") return { ratio: clampRatio(update) };
  const ratio = clampRatio(update.ratio);
  const normalized: RenderProgress = { ratio };
  if (update.phase !== undefined) normalized.phase = update.phase;
  if (update.message !== undefined) normalized.message = update.message;
  if (update.current !== undefined) normalized.current = update.current;
  if (update.total !== undefined) normalized.total = update.total;
  return normalized;
}

function errorRecord(value: unknown): RenderJobError {
  if (value instanceof RenderJobCancelledError) {
    return { name: value.name, message: value.message, code: value.code };
  }
  if (value instanceof Error) {
    const result: RenderJobError = { name: value.name, message: value.message };
    const code = (value as Error & { code?: unknown }).code;
    if (typeof code === "string") result.code = code;
    if (value.stack !== undefined) result.stack = value.stack;
    const details = (value as Error & { details?: unknown }).details;
    if (details !== undefined) result.details = details;
    return result;
  }
  if (typeof value === "string") return { name: "Error", message: value };
  try {
    return { name: "Error", message: JSON.stringify(value) };
  } catch {
    return { name: "Error", message: String(value) };
  }
}

function isAbortError(value: unknown): boolean {
  if (value instanceof RenderJobCancelledError) return true;
  if (!(value instanceof Error)) return false;
  const code = (value as Error & { code?: unknown }).code;
  return code === "ABORT_ERR" || value.name === "AbortError";
}

function cloneSnapshot(record: JobRecord): RenderJobSnapshot {
  const snapshot: RenderJobSnapshot = {
    id: record.id,
    status: record.status,
    state: record.status,
    progress: { ...record.progress },
    createdAt: record.createdAt,
  };
  if (record.kind !== undefined) snapshot.kind = record.kind;
  if (record.startedAt !== undefined) snapshot.startedAt = record.startedAt;
  if (record.completedAt !== undefined) snapshot.completedAt = record.completedAt;
  if (record.value !== undefined && record.status === "completed") {
    snapshot.result = record.value;
  }
  if (record.error !== undefined) snapshot.error = { ...record.error };
  if (record.metadata !== undefined) snapshot.metadata = { ...record.metadata };
  return snapshot;
}

function findTask<T>(spec: CommonJobSpec<T>): RenderJobTask<T> | undefined {
  return spec.run ?? spec.job ?? spec.execute ?? spec.task ?? spec.callback;
}

/**
 * A small FIFO scheduler for media work. The manager owns cancellation and
 * lifecycle state; the injected callback owns the actual render process.
 */
export class RenderJobManager {
  readonly concurrency: number;
  readonly maxConcurrent: number;
  private readonly jobs = new Map<string, JobRecord>();
  private readonly queue: JobRecord[] = [];
  private readonly update: ((snapshot: RenderJobSnapshot) => void) | undefined;
  private readonly injectedRender: RenderJobExecutor | undefined;
  private active = 0;
  private sequence = 0;

  constructor(options: RenderJobManagerOptions = {}) {
    const concurrency = options.concurrency ?? options.maxConcurrent ?? options.maxConcurrency ?? 1;
    if (!Number.isInteger(concurrency) || concurrency < 1) {
      throw new Error(`RenderJobManager concurrency must be a positive integer, got ${concurrency}`);
    }
    this.concurrency = concurrency;
    this.maxConcurrent = concurrency;
    this.update = options.onUpdate;
    this.injectedRender = options.render ?? options.renderer;
  }

  start<T>(task: RenderJobTask<T>, options?: Omit<GenericRenderJobSpec<T>, "run" | "job" | "execute">): string;
  start<T>(spec: RenderJobSpec<T>): string;
  start<T>(
    taskOrSpec: RenderJobTask<T> | RenderJobSpec<T>,
    options: Omit<GenericRenderJobSpec<T>, "run" | "job" | "execute"> = {},
  ): string {
    const spec: RenderJobSpec<T> =
      typeof taskOrSpec === "function"
        ? { ...options, run: taskOrSpec }
        : taskOrSpec;
    const task = this.taskForSpec(spec);
    if (typeof task !== "function") {
      throw new TypeError("RenderJobManager.start requires a render callback");
    }
    const id = this.uniqueId(spec.id);
    const now = Date.now();
    let resolvePromise: (value: unknown) => void = () => undefined;
    let rejectPromise: (reason?: unknown) => void = () => undefined;
    const promise = new Promise<unknown>((resolve, reject) => {
      resolvePromise = resolve;
      rejectPromise = reject;
    });
    // A host may poll status without ever awaiting result(). Attach a noop
    // rejection observer so cancellation/failure is not reported as an
    // unhandled process-level rejection; result() still returns this promise.
    void promise.catch(() => undefined);
    const record: JobRecord = {
      id,
      task: task as RenderJobTask<unknown>,
      controller: new AbortController(),
      status: "queued",
      progress: { ratio: 0, phase: "queued" },
      createdAt: now,
      cancelRequested: false,
      resolve: resolvePromise,
      reject: rejectPromise,
      promise,
      listeners: new Set(),
    };
    if (spec.kind !== undefined) record.kind = spec.kind;
    if (spec.onProgress !== undefined) record.callback = spec.onProgress;
    if (spec.metadata !== undefined) record.metadata = { ...spec.metadata };
    this.jobs.set(id, record);
    this.queue.push(record);
    this.notify(record);
    this.pump();
    return id;
  }

  /** Queue a v1 render, with an optional callback for deterministic hosts/tests. */
  startV1<T = void>(
    timeline: Timeline,
    options: RenderOptions,
    callback?: RenderV1Task<T>,
    jobOptions: Omit<GenericRenderJobSpec<T>, "run" | "job" | "execute" | "kind"> = {},
  ): string {
    const run: RenderJobTask<T> = (context) => {
      if (callback) return callback(timeline, options, context);
      if (context.signal.aborted) throw new RenderJobCancelledError();
      const renderOptions: RenderOptions = {
        ...options,
        onProgress: (ratio) => context.reportProgress({ ratio, phase: "render" }),
        signal: context.signal,
      };
      return render(timeline, renderOptions) as Promise<T>;
    };
    return this.start({ ...jobOptions, kind: "v1", run });
  }

  /** Queue a v2 render, with an optional callback for deterministic hosts/tests. */
  startV2<T = void>(
    project: Project,
    options: RenderV2Options,
    callback?: RenderV2Task<T>,
    jobOptions: Omit<GenericRenderJobSpec<T>, "run" | "job" | "execute" | "kind"> = {},
  ): string {
    const run: RenderJobTask<T> = (context) => {
      if (callback) return callback(project, options, context);
      if (context.signal.aborted) throw new RenderJobCancelledError();
      const renderOptions: RenderV2Options = {
        ...options,
        onProgress: (ratio) => context.reportProgress({ ratio, phase: "render" }),
        signal: context.signal,
      };
      return renderV2(project, renderOptions) as Promise<T>;
    };
    return this.start({ ...jobOptions, kind: "v2", run });
  }

  status<T = unknown>(id: string): RenderJobSnapshot<T> | undefined {
    const record = this.jobs.get(id);
    return record ? (cloneSnapshot(record) as RenderJobSnapshot<T>) : undefined;
  }

  /** Alias for status(). */
  getStatus<T = unknown>(id: string): RenderJobSnapshot<T> | undefined {
    return this.status<T>(id);
  }

  /**
   * Resolve with the callback's result, or reject with the original structured
   * error/cancellation once the job reaches a terminal state.
   */
  result<T = unknown>(id: string): Promise<T> {
    const record = this.jobs.get(id);
    if (!record) return Promise.reject(new Error(`Unknown render job: ${id}`));
    return record.promise as Promise<T>;
  }

  /** A non-throwing result lookup for hosts that poll status first. */
  getResult<T = unknown>(id: string): T | undefined {
    const record = this.jobs.get(id);
    return record?.status === "completed" ? (record.value as T) : undefined;
  }

  /** Cancel a queued or running job. Returns false for unknown/terminal jobs. */
  cancel(id: string, reason = "Render job was cancelled"): boolean {
    const record = this.jobs.get(id);
    if (!record || ["completed", "failed", "cancelled"].includes(record.status)) {
      return false;
    }
    record.cancelRequested = true;
    record.cancelReason = reason;
    record.controller.abort(new RenderJobCancelledError(reason));
    if (record.status === "queued") {
      this.finishCancelled(record);
      this.pump();
    }
    return true;
  }

  /** Subscribe to one job's state/progress updates. */
  subscribe(id: string, listener: (snapshot: RenderJobSnapshot) => void): () => void {
    const record = this.jobs.get(id);
    if (!record) return () => undefined;
    record.listeners.add(listener);
    try {
      listener(cloneSnapshot(record));
    } catch {
      /* Host listeners are observational and must not break the scheduler. */
    }
    return () => record.listeners.delete(listener);
  }

  /** Remove a terminal job's retained result/error from this manager. */
  forget(id: string): boolean {
    const record = this.jobs.get(id);
    if (!record || !["completed", "failed", "cancelled"].includes(record.status)) return false;
    return this.jobs.delete(id);
  }

  private taskForSpec<T>(spec: RenderJobSpec<T>): RenderJobTask<T> | undefined {
    const direct = findTask(spec);
    if (direct) return direct;
    if (this.injectedRender) {
      return (context) => this.injectedRender!(spec as RenderJobSpec, context) as T | Promise<T>;
    }
    if (spec.kind === "v1") {
      const v1 = spec as RenderV1JobSpec<T>;
      if (typeof v1.render === "function") {
        return (context) => v1.render!(v1.timeline, v1.options, context);
      }
      return (context) => {
        if (context.signal.aborted) throw new RenderJobCancelledError();
        const renderOptions: RenderOptions = {
          ...v1.options,
          onProgress: (ratio) => context.reportProgress({ ratio, phase: "render" }),
          signal: context.signal,
        };
        return render(v1.timeline, renderOptions) as unknown as T;
      };
    }
    if (spec.kind === "v2") {
      const v2 = spec as RenderV2JobSpec<T>;
      if (typeof v2.render === "function") {
        return (context) => v2.render!(v2.project, v2.options, context);
      }
      return (context) => {
        if (context.signal.aborted) throw new RenderJobCancelledError();
        const renderOptions: RenderV2Options = {
          ...v2.options,
          onProgress: (ratio) => context.reportProgress({ ratio, phase: "render" }),
          signal: context.signal,
        };
        return renderV2(v2.project, renderOptions) as unknown as T;
      };
    }
    return undefined;
  }

  private uniqueId(requested: string | undefined): string {
    if (requested !== undefined) {
      if (requested.length === 0) throw new Error("Render job id must not be empty");
      if (this.jobs.has(requested)) throw new Error(`Render job id already exists: ${requested}`);
      return requested;
    }
    let id = "render-" + ++this.sequence;
    while (this.jobs.has(id)) id = "render-" + ++this.sequence;
    return id;
  }

  private pump(): void {
    while (this.active < this.concurrency && this.queue.length > 0) {
      const record = this.queue.shift()!;
      if (record.status !== "queued") continue;
      if (record.cancelRequested) {
        this.finishCancelled(record);
        continue;
      }
      this.active += 1;
      record.status = "running";
      record.startedAt = Date.now();
      record.progress = { ...record.progress, phase: "render" };
      this.notify(record);
      void this.execute(record);
    }
  }

  private async execute(record: JobRecord): Promise<void> {
    const context: RenderJobContext = {
      id: record.id,
      signal: record.controller.signal,
      reportProgress: (update) => this.progress(record, update),
      report: (update) => this.progress(record, update),
    };
    try {
      const value = await record.task(context);
      if (record.cancelRequested || record.controller.signal.aborted) {
        this.finishCancelled(record);
      } else {
        record.value = value;
        record.progress = { ...record.progress, ratio: 1, phase: "complete" };
        this.finish(record, "completed");
        record.resolve(value);
      }
    } catch (cause) {
      if (record.cancelRequested || record.controller.signal.aborted || isAbortError(cause)) {
        this.finishCancelled(record, cause);
      } else {
        record.error = errorRecord(cause);
        this.finish(record, "failed");
        record.reject(cause);
      }
    } finally {
      this.active -= 1;
      this.pump();
    }
  }

  private progress(record: JobRecord, update: RenderProgressUpdate): void {
    if (record.status !== "running") return;
    record.progress = normalizeProgress(update);
    this.notify(record);
    if (record.callback) {
      try {
        record.callback({ ...record.progress });
      } catch {
        /* A progress observer cannot fail the render itself. */
      }
    }
  }

  private finishCancelled(record: JobRecord, cause?: unknown): void {
    if (["completed", "failed", "cancelled"].includes(record.status)) return;
    record.status = "cancelled";
    record.error = errorRecord(
      new RenderJobCancelledError(
        record.cancelReason ?? (cause instanceof Error ? cause.message : undefined),
      ),
    );
    record.completedAt = Date.now();
    this.notify(record);
    record.reject(new RenderJobCancelledError(record.error.message));
  }

  private finish(record: JobRecord, status: "completed" | "failed"): void {
    record.status = status;
    record.completedAt = Date.now();
    this.notify(record);
  }

  private notify(record: JobRecord): void {
    const snapshot = cloneSnapshot(record);
    try {
      this.update?.(snapshot);
    } catch {
      /* Host updates are observational. */
    }
    for (const listener of record.listeners) {
      try {
        listener(snapshot);
      } catch {
        /* One bad listener must not stop other listeners or the scheduler. */
      }
    }
  }
}

export const RenderJobScheduler = RenderJobManager;

export function createRenderJobManager(options: RenderJobManagerOptions = {}): RenderJobManager {
  return new RenderJobManager(options);
}
