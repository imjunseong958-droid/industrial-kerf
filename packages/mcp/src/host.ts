import { mkdir, readFile, rename, unlink, writeFile } from "node:fs/promises";
import { randomUUID } from "node:crypto";
import path from "node:path";
import {
  RenderJobManager,
  type RenderJobSnapshot,
  detectScenes,
  extractFrames,
  info,
  silenceDetect,
} from "@cut/media";
import {
  ProjectSchema,
  applyProjectCommand,
  applyProjectCommands,
  clipSourceId,
  clipTimelineDuration,
  clipTimelineStart,
  createProjectHistory,
  emptyProject,
  migrateV1ToV2,
  parseTimelineProject,
  type Project,
  type ProjectCommand,
  type ProjectHistory,
  type HistoryEntry,
} from "@cut/timeline";
import {
  CutMcpError,
  CUT_MCP_ERROR_CODES,
  type CutMcpHandlers,
  type CutMcpRenderHandlers,
  type TimelineApplyInput,
} from "./server.js";
import { commandFromLegacyOp } from "./legacy-ops.js";
import { createMediaHandlers } from "./host-media.js";
import { createVisionHandlers, resolveVisionSetup, type CutMcpVisionSetup } from "./host-vision.js";
import type { Yolo26TrackingConfig } from "@cut/vision";
import type { ProbeInfo } from "@cut/media";
import { createCutRuntimeClient, RuntimeClientError, type RuntimeClient } from "./runtime-client.js";

export const CUT_PROJECT_FILE = "project.cut.json";

export interface CutMcpProjectHostOptions {
  /** Project directory. All host reads and writes stay below this directory. */
  readonly dir: string;
  /** Optional injected render handlers; when omitted, project-scoped media job handlers are used. */
  readonly render?: CutMcpRenderHandlers;
  /** Optional scheduler used by the built-in render handlers. */
  readonly renderManager?: RenderJobManager;
  readonly cookies?: string;
  readonly fetchVideo?: (
    url: string,
    outDir: string,
    opts?: { cookies?: string },
  ) => Promise<{ path: string; subsPath?: string; title?: string }>;
  readonly info?: (src: string) => Promise<ProbeInfo>;
  readonly silenceDetect?: typeof silenceDetect;
  readonly detectScenes?: typeof detectScenes;
  readonly extractFrames?: typeof extractFrames;
  readonly vision?:
    | CutMcpVisionSetup
    | Yolo26TrackingConfig
    | { track(request: unknown): Promise<unknown> };
  /** Runtime is the canonical reducer/history owner. If omitted, the host
   * creates the deterministic cut-runtime client; there is no TS reducer fallback. */
  readonly runtime?: RuntimeClient;
}

export interface CutMcpProjectHost {
  readonly dir: string;
  readonly projectPath: string;
  readonly handlers: CutMcpHandlers;
  /** Return the current v2 history snapshot, loading the project lazily. */
  readonly history: () => Promise<ProjectHistory>;
  /** Serialize queued host work and close the in-memory state. */
  readonly close: () => Promise<void>;
}

interface HostState {
  history: ProjectHistory;
  /** Version found on disk before a first mutation migrates it to v2. */
  sourceVersion: 1 | 2;
}

interface ProjectSummary {
  version: 2;
  revision: number;
  sourceCount: number;
  trackCount: number;
  clipCount: number;
  duration: number;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function projectDuration(project: Project): number {
  let end = project.duration ?? 0;
  for (const track of project.tracks) {
    for (const clip of track.clips) {
      const duration = clipTimelineDuration(clip);
      if (duration === undefined) continue;
      end = Math.max(end, clipTimelineStart(clip) + duration);
    }
  }
  for (const gap of project.gaps) {
    end = Math.max(end, (gap.start ?? 0) + gap.duration);
  }
  return end;
}

function summarize(project: Project, revision: number): ProjectSummary {
  return {
    version: 2,
    revision,
    sourceCount: project.sources.length,
    trackCount: project.tracks.length,
    clipCount: project.tracks.reduce((total, track) => total + track.clips.length, 0),
    duration: projectDuration(project),
  };
}

function historyFromRuntime(snapshot: { project: Project; revision: number; canUndo: boolean; canRedo: boolean }): ProjectHistory {
  const base = createProjectHistory(snapshot.project, snapshot.revision);
  const placeholder: HistoryEntry = { command: { type: "set_metadata", patch: {} }, before: snapshot.project, after: snapshot.project };
  return {
    ...base,
    past: snapshot.canUndo ? [placeholder] : [],
    future: snapshot.canRedo ? [placeholder] : [],
  };
}

function projectFromDisk(value: unknown): { project: Project; sourceVersion: 1 | 2 } {
  const parsed = parseTimelineProject(value);
  if (parsed.version === 2) {
    return { project: ProjectSchema.parse(parsed), sourceVersion: 2 };
  }
  return {
    project: migrateV1ToV2(parsed),
    sourceVersion: 1,
  };
}

async function readDiskProject(projectPath: string): Promise<{
  project: Project;
  sourceVersion: 1 | 2;
}> {
  let raw: string;
  try {
    raw = await readFile(projectPath, "utf8");
  } catch (error) {
    const code = (error as NodeJS.ErrnoException).code;
    if (code === "ENOENT") {
      throw new Error(`no project found at ${projectPath}`);
    }
    throw new Error(
      `failed to read project at ${projectPath}: ${error instanceof Error ? error.message : String(error)}`,
      { cause: error },
    );
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw) as unknown;
  } catch (error) {
    throw new Error(
      `project at ${projectPath} contains malformed JSON: ${error instanceof Error ? error.message : String(error)}`,
      { cause: error },
    );
  }
  try {
    return projectFromDisk(parsed);
  } catch (error) {
    throw new Error(
      `project at ${projectPath} failed validation: ${error instanceof Error ? error.message : String(error)}`,
      { cause: error },
    );
  }
}

async function saveProjectAtomic(projectPath: string, project: Project): Promise<void> {
  const validated = ProjectSchema.parse(project);
  const parent = path.dirname(projectPath);
  await mkdir(parent, { recursive: true });
  const temp = path.join(
    parent,
    `.${path.basename(projectPath)}.${process.pid}.${randomUUID()}.tmp`,
  );
  try {
    await writeFile(temp, JSON.stringify(validated, null, 2), "utf8");
    await rename(temp, projectPath);
  } catch (error) {
    await unlink(temp).catch(() => {});
    throw error;
  }
}

function conflict(error: unknown): CutMcpError {
  const value = error as {
    code?: unknown;
    expectedRevision?: unknown;
    actualRevision?: unknown;
    message?: unknown;
  };
  if (value?.code === "REVISION_CONFLICT") {
    return new CutMcpError(
      CUT_MCP_ERROR_CODES.revisionConflict,
      typeof value.message === "string" ? value.message : "project revision conflict",
      {
        expectedRevision: value.expectedRevision,
        actualRevision: value.actualRevision,
      },
    );
  }
  if (typeof value?.code === "string") {
    return new CutMcpError(
      CUT_MCP_ERROR_CODES.runtimeError,
      typeof value.message === "string" ? value.message : "runtime request failed",
      {
        runtimeCode: value.code,
        ...(value.expectedRevision !== undefined ? { expectedRevision: value.expectedRevision } : {}),
        ...(value.actualRevision !== undefined ? { actualRevision: value.actualRevision } : {}),
        ...(error instanceof RuntimeClientError && error.details !== undefined ? { runtimeDetails: error.details } : {}),
      },
    );
  }
  return error instanceof CutMcpError
    ? error
    : new CutMcpError(
        CUT_MCP_ERROR_CODES.handlerFailed,
        error instanceof Error ? error.message : String(error),
      );
}

function canonicalCommands(
  project: Project,
  input: Record<string, unknown>,
): { single?: ProjectCommand; batch?: ProjectCommand[] } {
  const command = input.command;
  const commands = input.commands;
  if (isRecord(command)) return { single: command as ProjectCommand };
  if (Array.isArray(commands)) return { batch: commands as ProjectCommand[] };
  const op = input.op;
  const ops = input.ops;
  if (isRecord(op)) {
    const command = commandFromLegacyOp(project, op);
    if (command.type === "batch") return { batch: command.commands };
    return { single: command };
  }
  if (Array.isArray(ops)) {
    // Normalize each legacy operation against the project produced by the
    // preceding operation. This keeps generated clip ids and index/track
    // lookups deterministic inside one atomic transaction.
    let evolving = project;
    const batch: ProjectCommand[] = [];
    for (const value of ops) {
      if (!isRecord(value)) throw new Error("each legacy operation must be an object");
      const command = commandFromLegacyOp(evolving, value);
      if (command.type === "batch") {
        batch.push(...command.commands);
        evolving = applyProjectCommands(createProjectHistory(evolving), command.commands).project;
      } else {
        batch.push(command);
        evolving = applyProjectCommand(createProjectHistory(evolving), command).project;
      }
    }
    return {
      batch,
    };
  }
  throw new Error("timeline.apply requires one of command, commands, op, or ops");
}

function summarizeChanges(from: unknown, to: unknown): Array<{ path: string; from: unknown; to: unknown }> {
  const changes: Array<{ path: string; from: unknown; to: unknown }> = [];
  const walk = (left: unknown, right: unknown, currentPath: string): void => {
    if (Object.is(left, right)) return;
    if (isRecord(left) && isRecord(right)) {
      const keys = new Set([...Object.keys(left), ...Object.keys(right)]);
      for (const key of [...keys].sort()) walk(left[key], right[key], `${currentPath}.${key}`);
      return;
    }
    if (Array.isArray(left) && Array.isArray(right)) {
      const length = Math.max(left.length, right.length);
      for (let index = 0; index < length; index++) {
        walk(left[index], right[index], `${currentPath}[${index}]`);
      }
      return;
    }
    let same = false;
    try {
      same = JSON.stringify(left) === JSON.stringify(right);
    } catch {
      same = false;
    }
    if (!same) changes.push({ path: currentPath || "$", from: left, to: right });
  };
  walk(from, to, "");
  return changes;
}

function normalizeSnapshot(value: unknown): Project {
  const parsed = parseTimelineProject(value);
  return parsed.version === 2 ? ProjectSchema.parse(parsed) : migrateV1ToV2(parsed);
}

function stateProject(state: HostState): Project {
  return ProjectSchema.parse(state.history.project);
}

function cloneHistory(history: ProjectHistory): ProjectHistory {
  const clone = structuredClone({
    project: history.project,
    revision: history.revision,
    past: history.past,
    future: history.future,
  }) as ProjectHistory;
  Object.defineProperties(clone, {
    present: { enumerable: false, get: () => clone.project },
    undoStack: { enumerable: false, get: () => clone.past },
    redoStack: { enumerable: false, get: () => clone.future },
  });
  return clone;
}

/**
 * Render sources relative to the project directory, while leaving the
 * persisted project untouched. Output paths are generated below `dir` by the
 * job handlers, so callers cannot use render requests to select arbitrary
 * filesystem locations.
 */
function projectForRender(project: Project, dir: string): Project {
  return ProjectSchema.parse({
    ...structuredClone(project),
    sources: project.sources.map((source) => ({
      ...source,
      path: path.isAbsolute(source.path) ? source.path : path.resolve(dir, source.path),
      ...(typeof source.subs === "string"
        ? { subs: path.isAbsolute(source.subs) ? source.subs : path.resolve(dir, source.subs) }
        : {}),
    })),
    overlays: project.overlays.map((overlay) => resolveOverlayPath(overlay, dir)),
  });
}

function resolveOverlayPath<T extends Record<string, unknown>>(overlay: T, dir: string): T {
  const clone = structuredClone(overlay) as T;
  const pathValue = (clone as Record<string, unknown>).path;
  if (typeof pathValue === "string" && !path.isAbsolute(pathValue)) {
    (clone as Record<string, unknown>).path = path.resolve(dir, pathValue);
  }
  return clone;
}

function renderStatus(
  manager: RenderJobManager,
  jobId: string,
): RenderJobSnapshot {
  const status = manager.status(jobId);
  if (!status) throw new Error(`render job not found: ${jobId}`);
  return status;
}

/**
 * Build project-scoped handlers over the shared Rust runtime. The host owns
 * transport concerns and legacy import normalization; the runtime owns the
 * canonical project snapshot, revision, atomic history, and mutations.
 */
export function createCutMcpProjectHost(options: CutMcpProjectHostOptions): CutMcpProjectHost {
  const dir = path.resolve(options.dir);
  const projectPath = path.join(dir, CUT_PROJECT_FILE);
  const runtime = options.runtime ?? createCutRuntimeClient(dir);
  const renderManager = options.renderManager ?? new RenderJobManager();
  let state: HostState | undefined;
  let serial: Promise<void> = Promise.resolve();
  let closed = false;
  let runtimeReady = false;

  const ensureState = async (): Promise<HostState> => {
    if (closed) throw new Error("cut MCP project host is closed");
    if (!runtimeReady) {
      try {
        const normalized = await readDiskProject(projectPath);
        // Canonicalize legacy aliases/defaults before Rust opens the file.
        // This is an import adapter, never a second reducer/history owner.
        await saveProjectAtomic(projectPath, normalized.project);
        runtimeReady = true;
        const runtimeState = await runtime.getState();
        const loaded: HostState = {
          history: historyFromRuntime(runtimeState),
          sourceVersion: normalized.sourceVersion,
        };
        state = loaded;
        return loaded;
      } catch (error) {
        if (!(error instanceof Error) || !error.message.includes("no project found")) throw error;
        await saveProjectAtomic(projectPath, emptyProject());
        runtimeReady = true;
      }
    }
    const runtimeState = await runtime.getState();
    const loaded: HostState = { history: historyFromRuntime(runtimeState), sourceVersion: 2 };
    state = loaded;
    return loaded;
  };

  const enqueue = <T>(work: () => Promise<T>): Promise<T> => {
    const run = serial.then(work, work);
    serial = run.then(() => undefined, () => undefined);
    return run;
  };

  const historySnapshot = async (): Promise<ProjectHistory> => cloneHistory((await ensureState()).history);

  const inspect = async (input: { include?: "summary" | "full" }) =>
    enqueue(async () => {
      const current = await ensureState();
      const project = stateProject(current);
      return {
        ...summarize(project, current.history.revision),
        sourceVersion: current.sourceVersion,
        normalizedVersion: 2,
        ...(input.include === "full" ? { project } : {}),
      };
    });

  const query = async (input: { include?: "summary" | "full"; trackId?: string; clipId?: string }) =>
    enqueue(async () => {
      const current = await ensureState();
      const project = stateProject(current);
      const tracks = project.tracks
        .filter((track) => input.trackId === undefined || track.id === input.trackId)
        .map((track) => ({
          id: track.id,
          type: track.type,
          clips: track.clips
            .filter((clip) => input.clipId === undefined || clip.id === input.clipId)
            .map((clip) => ({
              id: clip.id,
              sourceId: clipSourceId(clip),
              timelineStart: clipTimelineStart(clip),
              duration: clipTimelineDuration(clip),
              in: clip.in,
              out: clip.out,
            })),
        }));
      return {
        revision: current.history.revision,
        tracks,
        ...(input.include === "full" ? { project } : {}),
      };
    });

  const validate = async (input: { strict?: boolean }) => {
    void input;
    try {
      const raw = await readFile(projectPath, "utf8");
      const parsed = JSON.parse(raw) as unknown;
      const normalized = projectFromDisk(parsed);
      return {
        valid: true,
        version: normalized.sourceVersion,
        normalizedVersion: 2,
        ...(normalized.sourceVersion === 1 ? { migrated: true } : {}),
      };
    } catch (error) {
      return {
        valid: false,
        error: error instanceof Error ? error.message : String(error),
      };
    }
  };

  const apply = async (input: TimelineApplyInput) =>
    enqueue(async () => {
        await ensureState();
        const current = await runtime.getState();
        let payload: { single?: ProjectCommand; batch?: ProjectCommand[] };
        try {
          payload = canonicalCommands(current.project, input as Record<string, unknown>);
        } catch (error) {
          throw conflict(error);
        }
        const command = payload.single ?? { type: "batch", commands: payload.batch ?? [] };
        const baseRevision = typeof input.expectedRevision === "number" ? input.expectedRevision : current.revision;
        const key = typeof (input as Record<string, unknown>).idempotencyKey === "string"
          ? String((input as Record<string, unknown>).idempotencyKey)
          : randomUUID();
        try {
          const next = await runtime.apply("mcp", baseRevision, key, command as ProjectCommand);
          return { project: next.project, revision: next.revision, canUndo: next.canUndo, canRedo: next.canRedo };
        } catch (error) {
          throw conflict(error);
        }
    });

  const historyMove = async (direction: "undo" | "redo", input: { steps?: number; expectedRevision?: number; idempotencyKey?: string }) =>
    enqueue(async () => {
        await ensureState();
        let current = await runtime.getState();
        const steps = input.steps ?? 1;
        if (!Number.isInteger(steps) || steps < 1 || steps > 100) throw new Error("steps must be an integer between 1 and 100");
        for (let index = 0; index < steps; index++) {
          const baseRevision = index === 0 && input.expectedRevision !== undefined ? input.expectedRevision : current.revision;
          const key = index === 0 && input.idempotencyKey
            ? input.idempotencyKey
            : randomUUID();
          current = direction === "undo"
            ? await runtime.undo("mcp", baseRevision, key)
            : await runtime.redo("mcp", baseRevision, key);
        }
        return { project: current.project, revision: current.revision, canUndo: current.canUndo, canRedo: current.canRedo };
    });

  const diff = async (input: { from?: Record<string, unknown>; to?: Record<string, unknown> }) =>
    enqueue(async () => {
      const current = await ensureState();
      const currentProject = stateProject(current);
      const from = input.from === undefined ? currentProject : normalizeSnapshot(input.from);
      const to = input.to === undefined ? currentProject : normalizeSnapshot(input.to);
      const changes = summarizeChanges(from, to);
      return { equal: changes.length === 0, changes, from, to, revision: current.history.revision };
    });

  const queueRender = async (draft: boolean) =>
    enqueue(async () => {
      const current = await ensureState();
      const revision = current.history.revision;
      const jobId = `render-${randomUUID()}`;
      const output = path.join(dir, `${jobId}.mp4`);
      const metadata = { output, revision, draft, sourceVersion: current.sourceVersion };
      const id = renderManager.startV2(
        projectForRender(stateProject(current), dir),
        { output, draft },
        undefined,
        { id: jobId, metadata },
      );
      return {
        jobId: id,
        output,
        revision,
        status: renderStatus(renderManager, id),
      };
    });

  const defaultRender: CutMcpRenderHandlers = {
    preview: () => queueRender(true),
    start: (input) => queueRender(input.draft ?? false),
    status: async (input) => ({ jobId: input.jobId, status: renderStatus(renderManager, input.jobId) }),
    cancel: async (input) => {
      const statusBefore = renderStatus(renderManager, input.jobId);
      const cancelled = renderManager.cancel(input.jobId);
      return {
        jobId: input.jobId,
        cancelled,
        status: renderStatus(renderManager, input.jobId),
        ...(statusBefore.status === "queued" ? { wasQueued: true } : {}),
      };
    },
    result: async (input) => {
      const result = await renderManager.result(input.jobId);
      const status = renderStatus(renderManager, input.jobId);
      return {
        jobId: input.jobId,
        status,
        ...(status.metadata?.output !== undefined ? { output: status.metadata.output } : {}),
        ...(result !== undefined ? { result } : {}),
      };
    },
  };

  const media = createMediaHandlers({
    dir,
    ...(options.cookies !== undefined ? { cookies: options.cookies } : {}),
    ...(options.fetchVideo !== undefined ? { fetchVideo: options.fetchVideo } : {}),
    ...(options.info !== undefined ? { info: options.info } : {}),
    ...(options.silenceDetect !== undefined ? { silenceDetect: options.silenceDetect } : {}),
    ...(options.detectScenes !== undefined ? { detectScenes: options.detectScenes } : {}),
    ...(options.extractFrames !== undefined ? { extractFrames: options.extractFrames } : {}),
    ensureProject: historySnapshot,
    runtime,
  });

  const visionSetup = resolveVisionSetup(dir, options.vision);
  const vision = createVisionHandlers({ dir, ...(visionSetup ? { setup: visionSetup } : {}) });

  const handlers: CutMcpHandlers = {
    project: { inspect, validate },
    timeline: {
      query,
      apply,
      undo: (input) => historyMove("undo", input),
      redo: (input) => historyMove("redo", input),
      diff,
    },
    media,
    ...(vision ? { vision } : {}),
    // A caller-provided render group may intentionally expose only a subset.
    // Otherwise use the project-scoped RenderJobManager handlers now that the
    // media scheduler is available.
    render: options.render ?? defaultRender,
  };

  return {
    dir,
    projectPath,
    handlers,
    history: () => enqueue(async () => cloneHistory((await ensureState()).history)),
    close: async () => {
      await serial;
      closed = true;
      runtime.close();
    },
  };
}
