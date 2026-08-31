import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import type { Project, ProjectSource } from "@cut/timeline";

/**
 * The editor has two explicit transports: Tauri's typed command bus for the
 * desktop runtime and HTTP for browser/MCP hosting.  They share semantic
 * payloads, but the desktop path never falls back to loopback HTTP.
 */

export type JsonObject = Record<string, unknown>;

export type { Project, TimelineClip, ProjectSource, OverlayV2 } from "@cut/timeline";
export type Source = ProjectSource & {
  name?: string;
  title?: string;
  url?: string;
  subtitle?: unknown;
  subtitles?: unknown;
  subtitlePath?: string;
  subtitlesPath?: string;
  subsPath?: string;
  subs?: unknown;
  captions?: unknown;
};

export type StatePayload = {
  project: Project;
  revision: number;
  canUndo: boolean;
  canRedo: boolean;
  duration: number;
  sources: Source[];
  pendingProposal?: PendingProposal;
};

export type ProposalChange = {
  path: string;
  kind: "added" | "removed" | "changed" | string;
  before: unknown;
  after: unknown;
};

export type PendingProposal =
  | {
      status: "pending";
      proposalId: string;
      baseRevision: number;
      command?: JsonObject;
      diff: ProposalChange[];
      diffTruncated?: boolean;
    }
  | {
      status: "accepted";
      proposalId: string;
      baseRevision: number;
      appliedRevision: number;
    }
  | {
      status: "rejected";
      proposalId: string;
      baseRevision: number;
    };

export type RenderQuality = "preview" | "full";

export type RenderJob = {
  jobId: string;
  status?: string;
  state?: string;
  outputUrl?: string;
  outputPath?: string;
  error?: string;
  [key: string]: unknown;
};

export type AgentEvent = JsonObject | string;

export type AgentModelOption = {
  id: string;
  label?: string;
  provider?: string;
  [key: string]: unknown;
};

export type AgentConfig = {
  provider?: string;
  model?: string;
  models?: AgentModelOption[];
  providers?: string[];
  options?: {
    providers?: string[];
    models?: Record<string, string[]>;
    [key: string]: unknown;
  };
  [key: string]: unknown;
};

export type AgentResponse = {
  text?: string;
  events?: AgentEvent[];
  state?: StatePayload | JsonObject;
  proposal?: PendingProposal;
  [key: string]: unknown;
};

export type ImportRequest = {
  path?: string;
  url?: string;
  file?: Blob;
};

export type ImportResponse = {
  sourceId?: string;
  clipId?: string;
  source?: Source;
  state?: StatePayload | JsonObject;
  subtitle?: unknown;
  subtitles?: unknown;
  captions?: unknown;
  [key: string]: unknown;
};

export class ApiError extends Error {
  readonly status: number;
  readonly code?: string;
  readonly details?: unknown;
  readonly payload?: unknown;

  constructor(message: string, options: { status: number; code?: string; details?: unknown; payload?: unknown }) {
    super(message);
    this.name = "ApiError";
    this.status = options.status;
    this.code = options.code;
    this.details = options.details;
    this.payload = options.payload;
  }

  get isConflict(): boolean {
    return this.status === 409 || this.code === "REVISION_CONFLICT";
  }
}

const configuredApiRoot = (import.meta.env.VITE_CUT_API_URL as string | undefined)?.replace(/\/$/, "") ?? "";

export function isDesktopRuntime(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export async function pickMediaFile(): Promise<string | undefined> {
  if (!isDesktopRuntime()) return undefined;
  const selected = await openFileDialog({
    multiple: false,
    directory: false,
    filters: [{ name: "Video", extensions: ["mp4", "mov", "mkv", "webm", "m4v", "avi"] }],
  });
  return typeof selected === "string" ? selected : undefined;
}

function idempotencyKey(): string {
  return globalThis.crypto?.randomUUID?.() ?? `ui-${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

async function desktopInvoke<T>(command: string, args: Record<string, unknown> = {}): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (error) {
    const payload = objectValue(error);
    const code = typeof payload?.code === "string" ? payload.code : undefined;
    const message = errorMessage(error, `Desktop runtime command failed: ${command}`);
    throw new ApiError(message, { status: code === "REVISION_CONFLICT" ? 409 : 500, code, payload: error });
  }
}

export function apiUrl(path: string): string {
  if (/^(?:https?:|blob:|data:)/i.test(path)) return path;
  const normalized = path.startsWith("/") ? path : `/${path}`;
  return `${configuredApiRoot}${normalized}`;
}

export function mediaUrl(sourceId: string, sourcePath?: string): string {
  if (isDesktopRuntime() && sourcePath) return convertFileSrc(sourcePath);
  return apiUrl(`/api/media/${encodeURIComponent(sourceId)}`);
}

export function outputUrl(jobId: string): string {
  return apiUrl(`/api/output/${encodeURIComponent(jobId)}`);
}

function objectValue(value: unknown): JsonObject | undefined {
  return typeof value === "object" && value !== null ? (value as JsonObject) : undefined;
}

function errorMessage(payload: unknown, fallback: string): string {
  const object = objectValue(payload);
  if (typeof object?.message === "string") return object.message;
  if (typeof object?.error === "string") return object.error;
  if (typeof object?.details === "string") return object.details;
  return fallback;
}

async function parseResponse(response: Response): Promise<unknown> {
  const raw = await response.text();
  if (!raw) return undefined;
  try {
    return JSON.parse(raw) as unknown;
  } catch {
    return raw;
  }
}

async function request<T>(path: string, init: RequestInit = {}): Promise<T> {
  let response: Response;
  try {
    const isMultipart = typeof FormData !== "undefined" && init.body instanceof FormData;
    response = await fetch(apiUrl(path), {
      ...init,
      headers: {
        Accept: "application/json",
        ...(init.body !== undefined && !isMultipart ? { "Content-Type": "application/json" } : {}),
        ...init.headers,
      },
    });
  } catch (error) {
    throw new ApiError(error instanceof Error ? error.message : "Unable to reach the Cut host", { status: 0 });
  }

  const payload = await parseResponse(response);
  if (!response.ok) {
    const object = objectValue(payload);
    throw new ApiError(errorMessage(payload, `${response.status} ${response.statusText}`), {
      status: response.status,
      code: typeof object?.code === "string" ? object.code : undefined,
      details: object?.details,
      payload,
    });
  }
  return payload as T;
}

function sourceList(value: unknown, project: Project): Source[] {
  if (Array.isArray(value)) return value.filter((source): source is Source => Boolean(objectValue(source)?.id));
  return Array.isArray(project.sources) ? project.sources : [];
}

/** Accept the state envelope directly and the same envelope nested under state. */
export function normalizeState(value: unknown): StatePayload {
  const root = objectValue(value);
  const candidate = objectValue(root?.state) ?? root;
  const projectValue = candidate?.project;
  const project = objectValue(projectValue) as Project | undefined;
  if (!project) throw new Error("Cut host returned an invalid project state");
  const sources = sourceList(candidate?.sources, project);
  const duration = typeof candidate?.duration === "number" && Number.isFinite(candidate.duration)
    ? candidate.duration
    : typeof project.duration === "number" && Number.isFinite(project.duration)
      ? project.duration
      : 0;
  const revision = typeof candidate?.revision === "number" ? candidate.revision : 0;
  const pendingProposal = normalizeProposal(candidate?.pendingProposal ?? root?.proposal);
  return {
    project,
    revision,
    canUndo: candidate?.canUndo === true,
    canRedo: candidate?.canRedo === true,
    duration,
    sources,
    ...(pendingProposal ? { pendingProposal } : {}),
  };
}

/** Parse the runtime's discriminated proposal lifecycle at the transport boundary. */
function normalizeProposal(value: unknown): PendingProposal | undefined {
  const object = objectValue(value);
  if (!object || typeof object.proposalId !== "string" || !object.proposalId.trim()) return undefined;
  const baseRevision = object.baseRevision;
  if (typeof baseRevision !== "number" || !Number.isInteger(baseRevision) || baseRevision < 0) return undefined;
  if (object.status === "pending") {
    if (!Array.isArray(object.diff)) return undefined;
    const diff = object.diff
      .map((item): ProposalChange | undefined => {
        const change = objectValue(item);
        if (!change || typeof change.path !== "string" || typeof change.kind !== "string") return undefined;
        return {
          path: change.path,
          kind: change.kind as ProposalChange["kind"],
          before: change.before,
          after: change.after,
        };
      })
      .filter((change): change is ProposalChange => Boolean(change));
    return {
      status: "pending",
      proposalId: object.proposalId,
      baseRevision,
      ...(objectValue(object.command) ? { command: object.command as JsonObject } : {}),
      diff,
      ...(typeof object.diffTruncated === "boolean" ? { diffTruncated: object.diffTruncated } : {}),
    };
  }
  if (object.status === "accepted" && typeof object.appliedRevision === "number" && Number.isInteger(object.appliedRevision) && object.appliedRevision >= 0) {
    return {
      status: "accepted",
      proposalId: object.proposalId,
      baseRevision,
      appliedRevision: object.appliedRevision,
    };
  }
  if (object.status === "rejected") {
    return { status: "rejected", proposalId: object.proposalId, baseRevision };
  }
  return undefined;
}

export function stateFromResponse(value: unknown): StatePayload | undefined {
  try {
    return normalizeState(value);
  } catch {
    return undefined;
  }
}

export function loadState(): Promise<StatePayload> {
  if (isDesktopRuntime()) return desktopInvoke<StatePayload>("desktop_get_state").then(normalizeState);
  return request<unknown>("/api/state").then(normalizeState);
}

export function sendCommand(command: JsonObject, expectedRevision: number): Promise<unknown> {
  if (isDesktopRuntime()) {
    return desktopInvoke<StatePayload>("desktop_apply_command", {
      request: { command, actor: "ui", baseRevision: expectedRevision, idempotencyKey: idempotencyKey() },
    });
  }
  return request<unknown>("/api/command", {
    method: "POST",
    body: JSON.stringify({ command, expectedRevision }),
  });
}

export function undo(expectedRevision: number): Promise<unknown> {
  if (isDesktopRuntime()) {
    return desktopInvoke<StatePayload>("desktop_undo", {
      request: { actor: "ui", baseRevision: expectedRevision, idempotencyKey: idempotencyKey() },
    });
  }
  return request<unknown>("/api/undo", {
    method: "POST",
    body: JSON.stringify({ expectedRevision }),
  });
}

export function redo(expectedRevision: number): Promise<unknown> {
  if (isDesktopRuntime()) {
    return desktopInvoke<StatePayload>("desktop_redo", {
      request: { actor: "ui", baseRevision: expectedRevision, idempotencyKey: idempotencyKey() },
    });
  }
  return request<unknown>("/api/redo", {
    method: "POST",
    body: JSON.stringify({ expectedRevision }),
  });
}

export function runAgent(prompt: string, baseRevision = 0, requestId = idempotencyKey()): Promise<AgentResponse> {
  if (isDesktopRuntime()) {
    return desktopInvoke<AgentResponse>("desktop_run_agent", {
      request: { prompt, baseRevision, idempotencyKey: requestId },
    });
  }
  return request<AgentResponse>("/api/agent", {
    method: "POST",
    body: JSON.stringify({ prompt }),
  });
}

export function acceptProposal(proposalId: string, baseRevision: number, requestId = idempotencyKey()): Promise<unknown> {
  if (isDesktopRuntime()) {
    return desktopInvoke<StatePayload>("desktop_accept_proposal", {
      request: { proposalId, actor: "ui", baseRevision, idempotencyKey: requestId },
    });
  }
  return request<unknown>("/api/agent/proposal/accept", {
    method: "POST",
    body: JSON.stringify({ proposalId, baseRevision, idempotencyKey: requestId }),
  });
}

export function rejectProposal(proposalId: string, baseRevision = 0, requestId = idempotencyKey()): Promise<unknown> {
  if (isDesktopRuntime()) {
    return desktopInvoke<StatePayload>("desktop_reject_proposal", {
      request: { proposalId, actor: "ui", baseRevision, idempotencyKey: requestId },
    });
  }
  return request<unknown>("/api/agent/proposal/reject", {
    method: "POST",
    body: JSON.stringify({ proposalId, baseRevision, idempotencyKey: requestId }),
  });
}

/** Read the provider/model choices exposed by the host. */
export function getAgentConfig(): Promise<AgentConfig> {
  if (isDesktopRuntime()) return Promise.resolve({ provider: "external", model: "CUT_AGENT_BIN", providers: ["external"], models: [{ id: "CUT_AGENT_BIN", label: "External agent adapter", provider: "external" }] });
  return request<unknown>("/api/agent/config", { method: "GET" }).then((value) => {
    const root = objectValue(value);
    const config = objectValue(root?.config) ?? root;
    if (!config) throw new Error("Cut host returned an invalid agent config");
    const options = objectValue(config.options);
    const optionModels = objectValue(options?.models);
    const nestedModels = optionModels
      ? Object.entries(optionModels).flatMap(([provider, values]) =>
          Array.isArray(values)
            ? values.map((model) => ({ id: model, label: model, provider }))
            : [],
        )
      : [];
    const rawModels = Array.isArray(config.models) ? config.models : nestedModels;
    const models = rawModels
      .map((model) => {
        if (typeof model === "string" && model.trim()) return { id: model.trim(), label: model.trim() };
        const option = objectValue(model);
        if (!option || typeof option.id !== "string" || !option.id.trim()) return undefined;
        return { ...option, id: option.id.trim() } as AgentModelOption;
      })
      .filter((model): model is AgentModelOption => Boolean(model));
    return {
      ...config,
      models,
      ...(Array.isArray(options?.providers) ? { providers: options.providers } : {}),
      ...(typeof config.provider === "string" ? { provider: config.provider } : {}),
      ...(typeof config.model === "string" ? { model: config.model } : {}),
    };
  });
}

/** Change the active host model. The host may return the refreshed config. */
export function updateAgentConfig(patch: { provider?: string; model?: string }): Promise<AgentConfig> {
  if (isDesktopRuntime()) return Promise.resolve({ provider: patch.provider ?? "external", model: patch.model ?? "CUT_AGENT_BIN", providers: [patch.provider ?? "external"], models: [{ id: patch.model ?? "CUT_AGENT_BIN", label: "External agent adapter", provider: patch.provider ?? "external" }] });
  return request<unknown>("/api/agent/config", {
    method: "POST",
    body: JSON.stringify(patch),
  }).then((value) => {
    const root = objectValue(value);
    const config = objectValue(root?.config) ?? root;
    return config ? (config as AgentConfig) : patch;
  });
}

/** Clear only the agent's conversation context; project state lives elsewhere. */
export function resetAgentSession(): Promise<void> {
  if (isDesktopRuntime()) return Promise.resolve();
  return request<unknown>("/api/agent/reset", { method: "POST" }).then(() => undefined);
}

/**
 * Import a local path, remote URL, or browser-selected file. FormData is
 * passed without a Content-Type header so the browser can add the multipart
 * boundary the host requires.
 */
export function importMedia(input: ImportRequest | string | Blob, baseRevision = 0, requestId = idempotencyKey()): Promise<ImportResponse> {
  if (typeof input === "string") {
    if (isDesktopRuntime()) return desktopInvoke<ImportResponse>("desktop_import_media", { request: { path: input.trim(), actor: "ui", baseRevision, idempotencyKey: requestId } });
    return request<ImportResponse>("/api/import", {
      method: "POST",
      body: JSON.stringify({ path: input }),
    });
  }
  if (isDesktopRuntime()) return Promise.reject(new Error("Desktop media import requires a native file path"));
  if (typeof Blob !== "undefined" && input instanceof Blob) {
    const form = new FormData();
    const file = input as Blob & { name?: string };
    form.append("file", input, file.name || "media");
    return request<ImportResponse>("/api/import", { method: "POST", body: form });
  }
  const requestInput = input as ImportRequest;
  if (isDesktopRuntime() && requestInput.url?.trim()) {
    return Promise.reject(new Error("YouTube URL import is not available in the desktop runtime; choose a local file"));
  }
  if (requestInput.file) {
    const form = new FormData();
    const file = requestInput.file as Blob & { name?: string };
    form.append("file", requestInput.file, file.name || "media");
    return request<ImportResponse>("/api/import", { method: "POST", body: form });
  }
  if (typeof requestInput.url === "string" && requestInput.url.trim()) {
    return request<ImportResponse>("/api/import", {
      method: "POST",
      body: JSON.stringify({ url: requestInput.url.trim() }),
    });
  }
  if (typeof requestInput.path === "string" && requestInput.path.trim()) {
    return request<ImportResponse>("/api/import", {
      method: "POST",
      body: JSON.stringify({ path: requestInput.path.trim() }),
    });
  }
  return Promise.reject(new Error("Choose a file or enter a YouTube URL"));
}

export function startRender(quality: RenderQuality): Promise<RenderJob> {
  if (isDesktopRuntime()) {
    return desktopInvoke<RenderJob>("desktop_start_render", { quality });
  }
  return request<unknown>("/api/render", {
    method: "POST",
    body: JSON.stringify({ quality }),
  }).then((value) => {
    const object = objectValue(value);
    const nestedJob = objectValue(object?.job);
    const jobId = nestedJob?.jobId ?? nestedJob?.id ?? object?.jobId ?? object?.id;
    if (typeof jobId !== "string" || jobId.length === 0) {
      throw new Error("Cut host did not return a render job id");
    }
    return normalizeRenderJob(jobId, { ...(object ?? {}), ...(nestedJob ?? {}) });
  });
}

export function getRender(jobId: string): Promise<RenderJob> {
  if (isDesktopRuntime()) {
    return desktopInvoke<RenderJob>("desktop_get_render", { jobId });
  }
  return request<unknown>(`/api/render/${encodeURIComponent(jobId)}`).then((value) => {
    return normalizeRenderJob(jobId, value);
  });
}

function renderError(value: unknown): string | undefined {
  if (typeof value === "string" && value.trim()) return value;
  const object = objectValue(value);
  if (!object) return undefined;
  for (const key of ["message", "detail", "error"]) {
    const candidate = object[key];
    if (typeof candidate === "string" && candidate.trim()) return candidate;
    if (candidate && typeof candidate === "object") {
      const nested = objectValue(candidate);
      if (typeof nested?.message === "string" && nested.message.trim()) return nested.message;
    }
  }
  return undefined;
}

/**
 * The host historically returned render status in two shapes: a flat job and
 * `{ status: { status, state, ... }, output }`. Normalize both at the API
 * boundary so polling can use a stable terminal status and never stringify an
 * object to `[object Object]`.
 */
export function normalizeRenderJob(jobId: string, value: unknown): RenderJob {
  const root = objectValue(value);
  if (!root) throw new Error("Cut host returned an invalid render status");
  const nestedStatus = objectValue(root.status);
  const statusValue = nestedStatus?.status ?? nestedStatus?.state ?? root.state ?? root.status;
  const stateValue = nestedStatus?.state ?? root.state;
  const status = typeof statusValue === "string" && statusValue.trim()
    ? statusValue.toLowerCase()
    : undefined;
  const state = typeof stateValue === "string" && stateValue.trim()
    ? stateValue.toLowerCase()
    : undefined;
  const output = root.outputUrl ?? root.output ?? nestedStatus?.outputUrl ?? nestedStatus?.output;
  const terminal = status !== undefined && ["completed", "complete", "done", "succeeded", "success"].includes(status);
  const outputPresent = typeof output === "string" && output.length > 0;
  const normalized: RenderJob = {
    ...root,
    jobId,
    ...(status ? { status } : {}),
    ...(state ? { state } : {}),
    ...(terminal || outputPresent ? { outputUrl: outputUrl(jobId) } : {}),
  };
  const error = renderError(nestedStatus?.error ?? root.error);
  if (error) normalized.error = error;
  // Do not leak the raw nested status object back to consumers expecting the
  // flat contract. `status` is always the normalized string when available.
  if (nestedStatus) delete normalized.status;
  if (status) normalized.status = status;
  return normalized;
}

export function resolveAssetUrl(value: string | undefined, fallbackJobId?: string, localPath?: string): string | undefined {
  if (isDesktopRuntime() && localPath) return convertFileSrc(localPath);
  if (value) return apiUrl(value);
  return fallbackJobId ? outputUrl(fallbackJobId) : undefined;
}
