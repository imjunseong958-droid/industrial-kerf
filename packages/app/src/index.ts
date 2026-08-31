import { createRequire } from "node:module";
import { randomUUID } from "node:crypto";
import { existsSync } from "node:fs";
import { mkdir, readFile, realpath, rm, stat, writeFile } from "node:fs/promises";
import { spawn } from "node:child_process";
import { homedir } from "node:os";
import { fileURLToPath } from "node:url";
import path from "node:path";
import type { Server } from "node:http";

import {
  AgentSession,
  DEFAULT_MODELS,
  DEFAULT_RESPONSES_MODEL,
  type AgentEvent,
  type AgentSessionOptions,
  type OpenAIApiMode,
  type ProviderName,
  type ReasoningEffort,
  type ToolMap,
} from "@cut/agent";
import { info, type ProbeInfo } from "@cut/media";
import { fetchVideo } from "@cut/fetcher";
import {
  createCutAgentTools as createHostAgentTools,
  createCutMcpProjectHost,
  createCutRuntimeClient,
  type CutMcpProjectHost,
  type TimelineApplyInput,
} from "@cut/mcp";
import { loadSkills, BUNDLED_SKILLS_DIR, type Skill } from "@cut/skills";
import {
  clipTimelineDuration,
  clipTimelineStart,
  emptyProject,
  type Project,
  type ProjectHistory,
} from "@cut/timeline";

const require = createRequire(import.meta.url);
const express = require("express") as ExpressFactory;

type RequestLike = {
  body?: unknown;
  headers: Record<string, string | string[] | undefined>;
  params: Record<string, string | undefined>;
  method?: string;
  on(event: string, listener: (...args: any[]) => void): unknown;
};

type ResponseLike = {
  headersSent?: boolean;
  writableEnded?: boolean;
  status(code: number): ResponseLike;
  json(value: unknown): ResponseLike;
  send(value: unknown): ResponseLike;
  end(value?: unknown): ResponseLike;
  setHeader(name: string, value: string | number): void;
  flushHeaders?: () => void;
  write(value: string): boolean;
  sendFile(file: string, options?: Record<string, unknown>, callback?: (error?: Error) => void): void;
};

type NextLike = (error?: unknown) => void;
type Middleware = (req: RequestLike, res: ResponseLike, next: NextLike) => unknown;
type RouteHandler = (req: RequestLike, res: ResponseLike, next: NextLike) => unknown;

type ExpressApplication = {
  use(...middleware: any[]): ExpressApplication;
  get(path: string | RegExp, ...handlers: RouteHandler[]): ExpressApplication;
  post(path: string, ...handlers: RouteHandler[]): ExpressApplication;
  listen(port: number, host: string, callback?: () => void): Server;
};

type ExpressFactory = (() => ExpressApplication) & {
  json(options?: Record<string, unknown>): Middleware;
  static(root: string): Middleware;
};

const DEFAULT_HOST = "127.0.0.1";
const DEFAULT_PORT = 0;
const PROJECT_FILE = "project.cut.json";
const IMPORTS_DIR = "imports";
const MAX_UPLOAD_BYTES = 512 * 1024 * 1024;

export type AgentSessionFactory = (options: AgentSessionOptions) => AgentSession | Promise<AgentSession>;

export interface CutAppOptions {
  readonly dir: string;
  readonly media?: string;
  readonly host?: string;
  readonly port?: number;
  readonly openBrowser?: boolean;
  readonly llm?: Omit<AgentSessionOptions, "tools">;
  readonly agentSession?: AgentSession;
  readonly agentSessionFactory?: AgentSessionFactory;
  readonly probe?: (source: string) => Promise<ProbeInfo>;
  /** Injectable URL downloader for embedders and tests. */
  readonly fetchVideo?: typeof fetchVideo;
  readonly guiDist?: string;
}

export interface StateResponse {
  project: Project;
  revision: number;
  canUndo: boolean;
  canRedo: boolean;
  duration: number;
  sources: Project["sources"];
}

export interface ImportResult {
  sourceId: string;
  clipId: string;
  source: Project["sources"][number];
  clip: Project["tracks"][number]["clips"][number];
  state: StateResponse;
  /** Path to downloaded/uploaded subtitles when available. */
  subtitlePath?: string;
  subsPath?: string;
  /** Parsed subtitle metadata and cue count for media-bin previews. */
  subtitle?: { path: string; format: string; cueCount: number };
  /** Original URL for URL imports. */
  url?: string;
  /** Human-readable title returned by yt-dlp. */
  title?: string;
}

/** Public, non-secret agent configuration returned by the config endpoint. */
export interface AgentConfigResponse {
  provider: ProviderName;
  model: string;
  /** Present only for OpenAI-compatible sessions. */
  apiMode?: OpenAIApiMode;
  /** Present only when the Responses API is selected. */
  reasoningEffort?: ReasoningEffort;
  options: {
    providers: ProviderName[];
    models: Record<ProviderName, string[]>;
    apiModes: OpenAIApiMode[];
    reasoningEfforts: ReasoningEffort[];
  };
}

export interface CutApp {
  readonly app: ExpressApplication;
  readonly host: CutMcpProjectHost;
  readonly importMedia: (sourcePath: string) => Promise<ImportResult>;
  readonly close: () => Promise<void>;
}

export interface StartedCutApp extends Omit<CutApp, "host"> {
  readonly url: string;
  readonly port: number;
  readonly host: string;
  readonly projectHost: CutMcpProjectHost;
  readonly server: Server;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function asRecord(value: unknown, label: string): Record<string, unknown> {
  if (!isRecord(value)) throw new HttpError(400, label + " must be an object");
  return value;
}

function asString(value: unknown, label: string): string {
  if (typeof value !== "string" || value.trim().length === 0) {
    throw new HttpError(400, label + " must be a non-empty string");
  }
  return value;
}

const AGENT_PROVIDERS: ProviderName[] = ["anthropic", "openai"];
const AGENT_API_MODES: OpenAIApiMode[] = ["chat", "responses"];
const AGENT_REASONING_EFFORTS: ReasoningEffort[] = [
  "none",
  "minimal",
  "low",
  "medium",
  "high",
  "xhigh",
  "max",
];

interface AgentRuntimeConfig {
  provider: ProviderName;
  model: string;
  apiMode?: OpenAIApiMode;
  reasoningEffort?: ReasoningEffort;
}

function isProviderName(value: unknown): value is ProviderName {
  return value === "anthropic" || value === "openai";
}

function isApiMode(value: unknown): value is OpenAIApiMode {
  return value === "chat" || value === "responses";
}

function isReasoningEffort(value: unknown): value is ReasoningEffort {
  return typeof value === "string" && AGENT_REASONING_EFFORTS.includes(value as ReasoningEffort);
}

function defaultAgentModel(provider: ProviderName, apiMode?: OpenAIApiMode): string {
  return provider === "openai" && apiMode === "responses"
    ? DEFAULT_RESPONSES_MODEL
    : DEFAULT_MODELS[provider];
}

function normalizeAgentRuntimeConfig(config: AgentRuntimeConfig): AgentRuntimeConfig {
  const provider = config.provider;
  const apiMode = provider === "openai" ? config.apiMode ?? "chat" : undefined;
  return {
    provider,
    model: config.model.trim() || defaultAgentModel(provider, apiMode),
    ...(apiMode ? { apiMode } : {}),
    ...(apiMode === "responses" && config.reasoningEffort
      ? { reasoningEffort: config.reasoningEffort }
      : {}),
  };
}

function runtimeConfigFromSession(
  session: AgentSession | undefined,
  options: CutAppOptions,
): AgentRuntimeConfig {
  const configuredProvider = options.llm?.provider;
  // Match AgentSession's precedence for injected clients so the config shown
  // before the first lazy turn describes the session that will be built.
  const fallbackProvider: ProviderName = options.llm?.client
    ? "anthropic"
    : options.llm?.apiMode === "responses" || options.llm?.openaiResponsesClient || options.llm?.openaiClient
      ? "openai"
      : isProviderName(configuredProvider)
        ? configuredProvider
        : "anthropic";
  const configuredMode = isApiMode(options.llm?.apiMode) ? options.llm?.apiMode : undefined;
  const fallbackMode: OpenAIApiMode | undefined = fallbackProvider === "openai"
    ? configuredMode ?? (options.llm?.openaiResponsesClient ? "responses" : "chat")
    : undefined;
  const fallbackModel = options.llm?.model?.trim() || defaultAgentModel(fallbackProvider, fallbackMode);

  const sessionProvider = session && isProviderName(session.provider) ? session.provider : undefined;
  const provider = sessionProvider ?? fallbackProvider;
  const sessionMode = session && isApiMode(session.apiMode) ? session.apiMode : undefined;
  const apiMode = provider === "openai"
    ? sessionMode ?? fallbackMode ?? "chat"
    : undefined;
  const model = session && typeof session.model === "string" && session.model.trim()
    ? session.model.trim()
    : (provider === fallbackProvider && !sessionProvider
      ? fallbackModel
      : defaultAgentModel(provider, apiMode));
  const reasoningEffort = session && isReasoningEffort(session.reasoningEffort)
    ? session.reasoningEffort
    : (apiMode === "responses" && !sessionProvider && isReasoningEffort(options.llm?.reasoningEffort)
      ? options.llm?.reasoningEffort
      : undefined);
  return normalizeAgentRuntimeConfig({
    provider,
    model,
    ...(apiMode ? { apiMode } : {}),
    ...(reasoningEffort ? { reasoningEffort } : {}),
  });
}

function agentConfigResponse(config: AgentRuntimeConfig): AgentConfigResponse {
  const models: Record<ProviderName, string[]> = {
    anthropic: [DEFAULT_MODELS.anthropic],
    openai: [DEFAULT_MODELS.openai, DEFAULT_RESPONSES_MODEL],
  };
  if (!models[config.provider].includes(config.model)) models[config.provider].push(config.model);
  return {
    ...config,
    options: {
      providers: [...AGENT_PROVIDERS],
      models,
      apiModes: [...AGENT_API_MODES],
      reasoningEfforts: [...AGENT_REASONING_EFFORTS],
    },
  };
}

function optionalRevision(value: unknown): number | undefined {
  if (value === undefined) return undefined;
  if (typeof value !== "number" || !Number.isInteger(value) || value < 0) {
    throw new HttpError(400, "expectedRevision must be a nonnegative integer");
  }
  return value;
}

function projectDuration(project: Project): number {
  let duration = typeof project.duration === "number" ? project.duration : 0;
  for (const track of project.tracks) {
    for (const clip of track.clips) {
      const clipDuration = clipTimelineDuration(clip);
      if (clipDuration !== undefined) {
        duration = Math.max(duration, clipTimelineStart(clip) + clipDuration);
      }
    }
  }
  for (const gap of project.gaps) duration = Math.max(duration, (gap.start ?? 0) + gap.duration);
  return duration;
}

function stateFromHistory(history: ProjectHistory): StateResponse {
  const project = structuredClone(history.project) as Project;
  return {
    project,
    revision: history.revision,
    canUndo: history.past.length > 0,
    canRedo: history.future.length > 0,
    duration: projectDuration(project),
    sources: structuredClone(project.sources),
  };
}

function resolveMediaPath(value: string): string {
  if (value.includes("\0")) throw new HttpError(400, "media path contains a NUL byte");
  return path.resolve(value);
}

function isContained(parent: string, candidate: string): boolean {
  const relative = path.relative(parent, candidate);
  return relative !== ".." && !relative.startsWith(".." + path.sep) && !path.isAbsolute(relative);
}

async function ensureProject(dir: string): Promise<void> {
  await mkdir(dir, { recursive: true });
  const projectPath = path.join(dir, PROJECT_FILE);
  try {
    await stat(projectPath);
    return;
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
  }
  await writeFile(projectPath, JSON.stringify(emptyProject(), null, 2), "utf8");
}

function projectInspect(host: CutMcpProjectHost) {
  const handler = host.handlers.project?.inspect;
  if (!handler) throw new Error("project.inspect capability is unavailable");
  return handler({ include: "full" }, { toolName: "project.inspect", capability: "project.inspect" });
}

function timelineQuery(host: CutMcpProjectHost, input: Record<string, unknown> = {}) {
  const handler = host.handlers.timeline?.query;
  if (!handler) throw new Error("timeline.query capability is unavailable");
  return handler(input, { toolName: "timeline.query", capability: "timeline.query" });
}

function timelineApply(host: CutMcpProjectHost, input: TimelineApplyInput) {
  const handler = host.handlers.timeline?.apply;
  if (!handler) throw new Error("timeline.apply capability is unavailable");
  return handler(input, { toolName: "timeline.apply", capability: "timeline.apply" });
}

function timelineHistory(host: CutMcpProjectHost, direction: "undo" | "redo", input: Record<string, unknown> = {}) {
  const handler = host.handlers.timeline?.[direction];
  if (!handler) throw new Error("timeline." + direction + " capability is unavailable");
  return handler(input, {
    toolName: "timeline." + direction,
    capability: ("timeline." + direction) as any,
  });
}

function renderHandler(host: CutMcpProjectHost, direction: "preview" | "start" | "status" | "result") {
  const handler = host.handlers.render?.[direction];
  if (!handler) throw new Error("render." + direction + " capability is unavailable");
  return handler as (input: any, context: any) => any;
}

function rejectReplaceProject(value: unknown): void {
  if (isRecord(value) && value.type === "replace_project") {
    throw new HttpError(400, "replace_project is not accepted on app command endpoints");
  }
}

function rejectReplaceProjectBatch(value: unknown): void {
  if (!Array.isArray(value)) return;
  for (const command of value) {
    rejectReplaceProject(command);
    if (isRecord(command) && command.type === "batch") rejectReplaceProjectBatch(command.commands);
  }
}

function wrapReplaceProjectGuard(tools: ToolMap): ToolMap {
  const guardExecute = (
    tool: NonNullable<ToolMap[string]>,
  ): NonNullable<ToolMap[string]> => ({
    ...tool,
    execute: async (input: unknown) => {
      if (isRecord(input)) {
        rejectReplaceProject(input.command);
        rejectReplaceProjectBatch(input.commands);
      }
      return tool.execute(input);
    },
  });
  return {
    ...tools,
    ...(tools.project_edit ? { project_edit: guardExecute(tools.project_edit) } : {}),
  };
}

/** Build the shared MCP agent tool map with replace_project blocked on edit endpoints. */
export function createCutAgentTools(
  host: CutMcpProjectHost,
  options?: { skills?: Skill[] },
): ToolMap {
  return wrapReplaceProjectGuard(createHostAgentTools(host, options) as ToolMap);
}

async function localFile(value: unknown): Promise<string> {
  const sourcePath = resolveMediaPath(asString(value, "path"));
  let details;
  try {
    details = await stat(sourcePath);
  } catch (error) {
    throw new HttpError(400, "media file is not readable: " + sourcePath, error);
  }
  if (!details.isFile()) throw new HttpError(400, "media path must point to a regular file");
  return sourcePath;
}

/** Resolve a generated import artifact and ensure it stays in its project dir. */
async function ownedImportFile(root: string, value: string, label: string): Promise<string> {
  const candidate = path.resolve(value);
  const resolvedRoot = await realpath(root);
  let resolvedCandidate: string;
  try {
    resolvedCandidate = await realpath(candidate);
  } catch (error) {
    throw new HttpError(400, `${label} is not readable`, error);
  }
  if (!isContained(resolvedRoot, resolvedCandidate)) {
    throw new HttpError(400, `${label} escaped the project imports directory`);
  }
  let details;
  try {
    details = await stat(resolvedCandidate);
  } catch (error) {
    throw new HttpError(400, `${label} is not readable`, error);
  }
  if (!details.isFile()) throw new HttpError(400, `${label} must be a regular file`);
  return resolvedCandidate;
}

interface SubtitleCue {
  start: number;
  end: number;
  text: string;
}

function subtitleTime(value: string): number | undefined {
  const pieces = value.trim().replace(",", ".").split(":");
  if (pieces.length < 2 || pieces.length > 3) return undefined;
  const seconds = Number(pieces.pop());
  const minutes = Number(pieces.pop());
  const hours = pieces.length === 1 ? Number(pieces[0]) : 0;
  if (![hours, minutes, seconds].every(Number.isFinite)) return undefined;
  const result = hours * 3600 + minutes * 60 + seconds;
  return result >= 0 ? result : undefined;
}

/** Parse the common VTT subset emitted by yt-dlp into timeline cues. */
function parseWebVtt(value: string): SubtitleCue[] {
  const lines = value.replace(/^\uFEFF/, "").split(/\r?\n/);
  const cues: SubtitleCue[] = [];
  let index = 0;
  while (index < lines.length) {
    const line = lines[index]?.trim() ?? "";
    if (!line || line === "WEBVTT" || line.startsWith("NOTE") || line.startsWith("STYLE") || line.startsWith("REGION")) {
      index += 1;
      continue;
    }
    const timing = line.match(/^(\S+)\s+-->\s+(\S+)/);
    if (!timing) {
      index += 1;
      continue;
    }
    const start = subtitleTime(timing[1]!);
    const end = subtitleTime(timing[2]!);
    index += 1;
    const text: string[] = [];
    while (index < lines.length && (lines[index]?.trim() ?? "") !== "") {
      text.push(lines[index]!.trim());
      index += 1;
    }
    if (start !== undefined && end !== undefined && end > start && text.length > 0) {
      cues.push({ start, end, text: text.join("\n") });
    }
    if (cues.length >= 50_000) break;
  }
  return cues;
}

interface MultipartFile {
  data: Buffer;
  filename: string;
}

function requestBody(req: RequestLike, maxBytes: number): Promise<Buffer> {
  return new Promise((resolve, reject) => {
    const chunks: Buffer[] = [];
    let total = 0;
    let settled = false;
    req.on("data", (chunk: Buffer | string) => {
      if (settled) return;
      const part = Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk);
      total += part.length;
      if (total > maxBytes) {
        settled = true;
        reject(new HttpError(413, "uploaded file is too large"));
        return;
      }
      chunks.push(part);
    });
    req.on("end", () => {
      if (settled) return;
      settled = true;
      resolve(Buffer.concat(chunks));
    });
    req.on("error", (error: unknown) => {
      if (settled) return;
      settled = true;
      reject(error);
    });
  });
}

function headerValue(headers: string, name: string): string | undefined {
  return headers
    .split(/\r?\n/)
    .find((line) => line.toLowerCase().startsWith(name.toLowerCase() + ":"))
    ?.slice(name.length + 1)
    .trim();
}

function dispositionParameter(value: string, name: string): string | undefined {
  const match = value.match(new RegExp(`(?:^|;)\\s*${name}=(?:"((?:\\\\.|[^"])*)"|([^;]*))`, "i"));
  const raw = match?.[1] ?? match?.[2];
  return raw?.replace(/\\([\\"])/g, "$1").trim();
}

/** Minimal multipart parser for the single browser `file` field. */
async function multipartFile(req: RequestLike): Promise<MultipartFile> {
  const contentType = req.headers["content-type"];
  const value = Array.isArray(contentType) ? contentType[0] : contentType;
  const boundary = value?.match(/boundary=(?:"([^"]+)"|([^;]+))/i)?.[1]
    ?? value?.match(/boundary=(?:"([^"]+)"|([^;]+))/i)?.[2]?.trim();
  if (!boundary) throw new HttpError(400, "multipart request is missing its boundary");
  const body = await requestBody(req, MAX_UPLOAD_BYTES);
  const marker = Buffer.from(`--${boundary}`);
  let cursor = body.indexOf(marker);
  while (cursor >= 0) {
    cursor += marker.length;
    if (body.subarray(cursor, cursor + 2).toString() === "--") break;
    if (body.subarray(cursor, cursor + 2).toString() === "\r\n") cursor += 2;
    const next = body.indexOf(Buffer.from(`\r\n--${boundary}`), cursor);
    if (next < 0) break;
    const part = body.subarray(cursor, next);
    const headerEnd = part.indexOf(Buffer.from("\r\n\r\n"));
    if (headerEnd < 0) {
      cursor = next + 2;
      continue;
    }
    const headers = part.subarray(0, headerEnd).toString("utf8");
    const bodyPart = part.subarray(headerEnd + 4);
    const disposition = headerValue(headers, "content-disposition");
    const name = disposition ? dispositionParameter(disposition, "name") : undefined;
    const filename = disposition ? dispositionParameter(disposition, "filename") : undefined;
    if (name === "file" && filename !== undefined) return { data: bodyPart, filename };
    cursor = next + 2;
  }
  throw new HttpError(400, "multipart request must include a file field");
}

function uploadExtension(filename: string): string {
  const extension = path.extname(path.basename(filename)).toLowerCase();
  return /^[.]([a-z0-9]{1,10})$/.test(extension) ? extension : ".media";
}

async function saveUploadedFile(host: CutMcpProjectHost, req: RequestLike): Promise<{ path: string; dir: string }> {
  const file = await multipartFile(req);
  const dir = path.join(host.dir, IMPORTS_DIR, randomUUID());
  await mkdir(dir, { recursive: true });
  const destination = path.join(dir, `upload${uploadExtension(file.filename)}`);
  try {
    await writeFile(destination, file.data, { flag: "wx" });
  } catch (error) {
    await rm(dir, { recursive: true, force: true }).catch(() => undefined);
    throw error;
  }
  return { path: destination, dir };
}

function uniqueId(prefix: string, used: Iterable<string>): string {
  const occupied = new Set(used);
  let value = prefix + "-" + randomUUID();
  while (occupied.has(value)) value = prefix + "-" + randomUUID();
  return value;
}

interface ImportExtras {
  subtitlePath?: string;
  url?: string;
  title?: string;
}

async function importIntoHost(
  host: CutMcpProjectHost,
  sourceInput: string,
  probe: (source: string) => Promise<ProbeInfo>,
  extras: ImportExtras = {},
): Promise<ImportResult> {
  const sourcePath = await localFile(sourceInput);
  const subtitlePath = extras.subtitlePath === undefined ? undefined : await localFile(extras.subtitlePath);
  const subtitleCues = subtitlePath === undefined
    ? []
    : await readFile(subtitlePath, "utf8").then(parseWebVtt, () => []);
  const metadata = await probe(sourcePath);
  if (metadata.duration === undefined || !Number.isFinite(metadata.duration) || metadata.duration <= 0) {
    throw new HttpError(400, "media probe did not return a positive duration");
  }
  const history = await host.history();
  const project = structuredClone(history.project) as Project;
  const timelineStart = projectDuration(project);
  const sourceMetadata: Record<string, unknown> = {
    duration: metadata.duration,
    ...(metadata.width !== undefined ? { width: metadata.width } : {}),
    ...(metadata.height !== undefined ? { height: metadata.height } : {}),
    ...(metadata.hasAudio !== undefined ? { hasAudio: metadata.hasAudio } : {}),
    ...(subtitlePath ? { subs: subtitlePath } : {}),
    metadata: {
      ...(metadata.fps !== undefined ? { fps: metadata.fps } : {}),
      ...(metadata.videoCodec !== undefined ? { videoCodec: metadata.videoCodec } : {}),
      ...(metadata.audioCodec !== undefined ? { audioCodec: metadata.audioCodec } : {}),
      ...(subtitlePath ? {
        subtitlePath,
        subtitleFormat: path.extname(subtitlePath).slice(1).toLowerCase() || "vtt",
        subtitleCueCount: subtitleCues.length,
      } : {}),
      ...(extras.url ? { url: extras.url } : {}),
      ...(extras.title ? { title: extras.title } : {}),
    },
  };
  const overlay = subtitleCues.length > 0 ? {
      id: uniqueId("captions", project.overlays.map((entry) => entry.id).filter((id): id is string => typeof id === "string")),
      type: "captions",
      cues: subtitleCues.map((cue) => ({
        ...cue,
        start: cue.start + timelineStart,
        end: cue.end + timelineStart,
      })),
      metadata: { path: subtitlePath },
    } : undefined;
  const importer = host.handlers.media?.import;
  if (!importer) throw new Error("media.import capability is unavailable");
  const imported = await importer({
    path: sourcePath,
    idempotencyKey: `app-import-${sourcePath}`,
    metadata: sourceMetadata,
    ...(overlay ? { overlay } : {}),
  }, { toolName: "media.import", capability: "media.import" });
  const updated = await host.history();
  const sourceId = String((imported as Record<string, unknown>).sourceId ?? "");
  const clipId = String((imported as Record<string, unknown>).clipId ?? "");
  const source = updated.project.sources.find((entry) => entry.id === sourceId);
  const clip = updated.project.tracks.flatMap((track) => track.clips).find((entry) => entry.id === clipId);
  if (!source || !clip) throw new Error("runtime import returned an incomplete source transaction");
  return {
    sourceId,
    clipId,
    source: structuredClone(source),
    clip: structuredClone(clip),
    state: stateFromHistory(updated),
    ...(subtitlePath ? {
      subtitlePath,
      subsPath: subtitlePath,
      subtitle: { path: subtitlePath, format: path.extname(subtitlePath).slice(1).toLowerCase() || "vtt", cueCount: subtitleCues.length },
    } : {}),
    ...(extras.url ? { url: extras.url } : {}),
    ...(extras.title ? { title: extras.title } : {}),
  };
}

class HttpError extends Error {
  readonly code: string | undefined;
  readonly details: unknown;
  constructor(readonly status: number, message: string, cause?: unknown, code?: string) {
    super(message, cause === undefined ? undefined : { cause });
    this.name = "HttpError";
    this.code = code;
    this.details = cause instanceof Error ? undefined : cause;
  }
}

function jsonError(error: unknown): Record<string, unknown> {
  if (error instanceof HttpError) {
    return {
      error: error.message,
      ...(error.code ? { code: error.code } : {}),
      ...(error.details !== undefined ? { details: error.details } : {}),
    };
  }
  return { error: error instanceof Error ? error.message : String(error) };
}

function withErrors(handler: (req: RequestLike, res: ResponseLike) => Promise<void>): RouteHandler {
  return (req, res, next) => void handler(req, res).catch(next);
}

function sendError(res: ResponseLike, error: unknown): void {
  res.status(error instanceof HttpError ? error.status : 500).json(jsonError(error));
}

function acceptNdjson(req: RequestLike): boolean {
  const accept = req.headers.accept;
  return (Array.isArray(accept) ? accept.join(",") : accept ?? "").includes("application/x-ndjson");
}

function statusRecord(value: unknown): Record<string, unknown> {
  return isRecord(value) ? value : { value };
}

function sendFile(res: ResponseLike, file: string): Promise<void> {
  return new Promise((resolve, reject) => {
    res.sendFile(file, { acceptRanges: true }, (error) => error ? reject(error) : resolve());
  });
}

function guiDistPath(options: CutAppOptions): string | undefined {
  const candidates = [
    options.guiDist,
    path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../gui/dist"),
    path.resolve(process.cwd(), "packages/gui/dist"),
  ].filter((candidate): candidate is string => typeof candidate === "string");
  return candidates.find((candidate) => existsSync(path.join(candidate, "index.html")));
}

function openBrowser(url: string): void {
  const command = process.platform === "darwin" ? "open" : process.platform === "win32" ? "cmd" : "xdg-open";
  const args = process.platform === "win32" ? ["/c", "start", "", url] : [url];
  try {
    const child = spawn(command, args, { detached: true, stdio: "ignore", shell: false });
    child.unref();
  } catch {
    // Browser launching is best effort.
  }
}

async function renderJobResult(
  host: CutMcpProjectHost,
  jobId: string,
  knownJobs: ReadonlySet<string>,
): Promise<{ jobId: string; status: Record<string, unknown>; output?: string; result?: unknown }> {
  if (!knownJobs.has(jobId)) throw new HttpError(404, "render job not found");
  const statusValue = await renderHandler(host, "status")(
    { jobId },
    { toolName: "render.status", capability: "render.status" },
  );
  const statusObject = statusRecord(statusValue);
  const nested = isRecord(statusObject.status) ? statusObject.status : statusObject;
  const metadata = isRecord(statusObject.metadata) ? statusObject.metadata : undefined;
  const output = typeof statusObject.output === "string"
    ? statusObject.output
    : typeof metadata?.output === "string" ? metadata.output : undefined;
  if (nested.status === "completed") {
    const resultValue = await renderHandler(host, "result")(
      { jobId },
      { toolName: "render.result", capability: "render.result" },
    );
    const resultObject = statusRecord(resultValue);
    return {
      jobId,
      status: nested,
      ...(output ? { output } : typeof resultObject.output === "string" ? { output: resultObject.output } : {}),
      ...(resultObject.result !== undefined ? { result: resultObject.result } : {}),
    };
  }
  return { jobId, status: nested, ...(output ? { output } : {}) };
}

/** Create an embeddable local HTTP application without starting a listener. */
export async function createCutApp(options: CutAppOptions): Promise<CutApp> {
  const dir = path.resolve(asString(options.dir, "dir"));
  await ensureProject(dir);
  // Rust owns the canonical project reducer/history. Binary discovery is
  // deterministic and fails with an actionable error; TS never becomes a
  // second mutation authority.
  const runtime = createCutRuntimeClient(dir);
  const projectHost = createCutMcpProjectHost({
    dir,
    fetchVideo: options.fetchVideo ?? fetchVideo,
    info: options.probe ?? info,
    runtime,
  });
  const probe = options.probe ?? info;
  let closed = false;
  let agentSession: AgentSession | undefined = options.agentSession;
  let agentSessionPromise: Promise<AgentSession> | undefined;
  let agentSerial: Promise<void> = Promise.resolve();
  // Configuration is kept separately from the lazy session so the GUI can
  // inspect and change it even when no API key is available yet. A config
  // change invalidates the session and therefore starts a fresh context on
  // the next agent turn; it does not eagerly construct an LLM client.
  let agentConfig = runtimeConfigFromSession(agentSession, options);
  const knownJobs = new Set<string>();

  const state = async (): Promise<StateResponse> => stateFromHistory(await projectHost.history());
  const importMedia = (sourcePath: string): Promise<ImportResult> =>
    importIntoHost(projectHost, sourcePath, probe);
  const downloadVideo = options.fetchVideo ?? fetchVideo;

  const importUrl = async (value: unknown): Promise<ImportResult> => {
    const url = asString(value, "url").trim();
    let parsed: URL;
    try {
      parsed = new URL(url);
    } catch (error) {
      throw new HttpError(400, "url must be an absolute http(s) URL", error);
    }
    if (parsed.protocol !== "http:" && parsed.protocol !== "https:") {
      throw new HttpError(400, "url must use http or https");
    }
    const importDir = path.join(projectHost.dir, IMPORTS_DIR, randomUUID());
    await mkdir(importDir, { recursive: true });
    try {
      const fetched = await downloadVideo(url, importDir);
      const mediaPath = await ownedImportFile(importDir, fetched.path, "downloaded media");
      const subtitlePath = fetched.subsPath === undefined
        ? undefined
        : await ownedImportFile(importDir, fetched.subsPath, "downloaded subtitles");
      return await importIntoHost(projectHost, mediaPath, probe, {
        ...(subtitlePath ? { subtitlePath } : {}),
        url,
        ...(fetched.title ? { title: fetched.title } : {}),
      });
    } catch (error) {
      await rm(importDir, { recursive: true, force: true }).catch(() => undefined);
      throw error;
    }
  };

  const importUpload = async (req: RequestLike): Promise<ImportResult> => {
    const uploaded = await saveUploadedFile(projectHost, req);
    try {
      return await importIntoHost(projectHost, uploaded.path, probe);
    } catch (error) {
      await rm(uploaded.dir, { recursive: true, force: true }).catch(() => undefined);
      throw error;
    }
  };

  const getSession = async (): Promise<AgentSession> => {
    if (agentSession) return agentSession;
    agentSessionPromise ??= (async () => {
      const skills = await loadSkills([
        BUNDLED_SKILLS_DIR,
        path.join(homedir(), ".cut", "skills"),
        path.join(dir, ".cut", "skills"),
      ]);
      const configured: AgentSessionOptions = {
        ...(options.llm ?? {}),
        provider: agentConfig.provider,
        model: agentConfig.model,
        ...(agentConfig.provider === "openai"
          ? {
              apiMode: agentConfig.apiMode,
              ...(agentConfig.reasoningEffort ? { reasoningEffort: agentConfig.reasoningEffort } : {}),
            }
          : {}),
        tools: createCutAgentTools(projectHost, { skills }),
        skills,
      };
      // Injected test clients belong to one provider/mode.
      // Do not let them silently override a user-selected replacement config.
      if (configured.provider === "anthropic") {
        delete configured.openaiClient;
        delete configured.openaiResponsesClient;
        delete configured.apiMode;
        delete configured.reasoningEffort;
      } else {
        delete configured.client;
        if (configured.apiMode === "responses") delete configured.openaiClient;
        else delete configured.openaiResponsesClient;
      }
      const session = options.agentSessionFactory
        ? await options.agentSessionFactory(configured)
        : new AgentSession(configured);
      agentSession = session;
      return session;
    })();
    return agentSessionPromise;
  };

  const runAgent = async (
    prompt: string,
    onEvent: (event: AgentEvent) => void,
  ): Promise<{ text: string; events: AgentEvent[] }> => {
    let text = "";
    const events: AgentEvent[] = [];
    let resolveResult!: (result: { text: string; events: AgentEvent[] }) => void;
    let rejectResult!: (error: unknown) => void;
    const result = new Promise<{ text: string; events: AgentEvent[] }>((resolve, reject) => {
      resolveResult = resolve;
      rejectResult = reject;
    });
    const run = agentSerial.then(async () => {
      const session = await getSession();
      const unsubscribe = session.onEvent((event) => {
        events.push(event);
        onEvent(event);
      });
      try {
        text = await session.run(prompt);
        resolveResult({ text, events });
      } catch (error) {
        rejectResult(error);
      } finally {
        unsubscribe();
      }
    }, rejectResult);
    agentSerial = run.then(() => undefined, () => undefined);
    return result;
  };

  /**
   * Reset only the reusable agent session. Queueing this behind agentSerial is
   * important: a new-chat request must never clear messages while a turn is
   * still appending tool results, and it must not touch the project host.
   */
  const resetAgentSession = async (): Promise<void> => {
    const reset = agentSerial.then(async () => {
      const session = agentSession ?? (agentSessionPromise ? await agentSessionPromise : undefined);
      if (session) session.messages = [];
    });
    agentSerial = reset.then(() => undefined, () => undefined);
    await reset;
  };

  /**
   * Apply a config update after all queued agent turns complete. The old
   * session history is discarded, while project/timeline state is untouched.
   * Session creation remains lazy so opening the GUI never requires a key.
   */
  const updateAgentConfig = async (body: Record<string, unknown>): Promise<AgentConfigResponse> => {
    const update = agentSerial.then(async () => {
      if (body.apiKey !== undefined || body.baseUrl !== undefined || body.baseURL !== undefined) {
        throw new HttpError(400, "agent credentials and base URLs cannot be changed via this endpoint");
      }

      const current = agentConfig;
      let provider = current.provider;
      if (body.provider !== undefined) {
        if (!isProviderName(body.provider)) {
          throw new HttpError(400, "provider must be anthropic or openai");
        }
        provider = body.provider;
      }
      let requestedMode: OpenAIApiMode | undefined;
      if (body.apiMode !== undefined) {
        if (!isApiMode(body.apiMode)) throw new HttpError(400, "apiMode must be chat or responses");
        requestedMode = body.apiMode;
      }
      const modeChangedByProvider = provider !== current.provider;
      const apiMode = provider === "openai"
        ? requestedMode ?? (modeChangedByProvider ? "chat" : current.apiMode ?? "chat")
        : undefined;

      let model: string;
      if (body.model !== undefined) {
        model = asString(body.model, "model").trim();
      } else if (modeChangedByProvider || apiMode !== current.apiMode) {
        model = defaultAgentModel(provider, apiMode);
      } else {
        model = current.model;
      }

      let requestedReasoning: ReasoningEffort | undefined;
      const reasoningValue = body.reasoningEffort ?? body.reasoning;
      if (reasoningValue !== undefined) {
        if (!isReasoningEffort(reasoningValue)) {
          throw new HttpError(400, "reasoningEffort is not supported");
        }
        requestedReasoning = reasoningValue;
      }
      const reasoningEffort = apiMode === "responses"
        ? requestedReasoning ?? (apiMode === current.apiMode ? current.reasoningEffort : undefined)
        : undefined;
      const next = normalizeAgentRuntimeConfig({
        provider,
        model,
        ...(apiMode ? { apiMode } : {}),
        ...(reasoningEffort ? { reasoningEffort } : {}),
      });

      const changed = next.provider !== current.provider
        || next.model !== current.model
        || next.apiMode !== current.apiMode
        || next.reasoningEffort !== current.reasoningEffort;
      if (changed && options.agentSession && !options.agentSessionFactory) {
        throw new HttpError(409, "the injected agent session cannot be replaced");
      }
      if (changed) {
        const oldSession = agentSession ?? (agentSessionPromise ? await agentSessionPromise : undefined);
        if (oldSession) {
          oldSession.messages = [];
          oldSession.abort();
        }
        agentSession = undefined;
        agentSessionPromise = undefined;
        agentConfig = next;
      }
      return agentConfigResponse(agentConfig);
    });
    agentSerial = update.then(() => undefined, () => undefined);
    return update;
  };

  const app = express();
  app.use(express.json({ limit: "2mb" }));

  app.get("/api/state", withErrors(async (_req, res) => {
    res.json(await state());
  }));

  app.post("/api/command", withErrors(async (req, res) => {
    const body = asRecord(req.body, "request body");
    const command = asRecord(body.command, "command");
    rejectReplaceProject(command);
    const expectedRevision = body.expectedRevision === undefined
      ? undefined
      : optionalRevision(body.expectedRevision);
    const result = await timelineApply(projectHost, {
      command,
      ...(expectedRevision !== undefined ? { expectedRevision } : {}),
    });
    res.json(result);
  }));

  for (const direction of ["undo", "redo"] as const) {
    app.post("/api/" + direction, withErrors(async (req, res) => {
      const body = req.body === undefined ? {} : asRecord(req.body, "request body");
      const expectedRevision = body.expectedRevision === undefined
        ? undefined
        : optionalRevision(body.expectedRevision);
      const steps = body.steps;
      if (steps !== undefined && (
        typeof steps !== "number" || !Number.isInteger(steps) || steps < 1 || steps > 100
      )) {
        throw new HttpError(400, "steps must be an integer between 1 and 100");
      }
      res.json(await timelineHistory(projectHost, direction, {
        ...(steps !== undefined ? { steps } : {}),
        ...(expectedRevision !== undefined ? { expectedRevision } : {}),
      }));
    }));
  }

  app.post("/api/import", withErrors(async (req, res) => {
    const contentType = req.headers["content-type"];
    const type = Array.isArray(contentType) ? contentType[0] : contentType;
    if (type?.toLowerCase().startsWith("multipart/form-data")) {
      res.status(201).json(await importUpload(req));
      return;
    }
    const body = asRecord(req.body, "request body");
    if (body.path !== undefined) {
      res.status(201).json(await importMedia(asString(body.path, "path")));
      return;
    }
    if (body.url !== undefined) {
      res.status(201).json(await importUrl(body.url));
      return;
    }
    throw new HttpError(400, "request body must contain path, url, or multipart file");
  }));

  app.post("/api/render", withErrors(async (req, res) => {
    const body = req.body === undefined ? {} : asRecord(req.body, "request body");
    const quality = body.quality ?? "full";
    if (quality !== "preview" && quality !== "full") {
      throw new HttpError(400, "quality must be preview or full");
    }
    const preview = quality === "preview";
    const result = await renderHandler(projectHost, preview ? "preview" : "start")(
      preview ? { draft: true } : { draft: false },
      {
        toolName: preview ? "render.preview" : "render.start",
        capability: preview ? "render.preview" : "render.start",
      },
    );
    const resultObject = statusRecord(result);
    if (typeof resultObject.jobId === "string") knownJobs.add(resultObject.jobId);
    res.status(202).json(result);
  }));

  app.get("/api/render/:jobId", withErrors(async (req, res) => {
    res.json(await renderJobResult(
      projectHost,
      asString(req.params.jobId, "jobId"),
      knownJobs,
    ));
  }));

  app.get("/api/media/:sourceId", withErrors(async (req, res) => {
    const current = await state();
    const source = current.project.sources.find((candidate) => candidate.id === asString(req.params.sourceId, "sourceId"));
    if (!source) throw new HttpError(404, "media source not found");
    const sourcePath = path.isAbsolute(source.path)
      ? source.path
      : path.resolve(projectHost.dir, source.path);
    try {
      const details = await stat(sourcePath);
      if (!details.isFile()) throw new Error("not a regular file");
    } catch {
      throw new HttpError(404, "media source file is unavailable");
    }
    await sendFile(res, sourcePath);
  }));

  app.get("/api/output/:jobId", withErrors(async (req, res) => {
    const job = await renderJobResult(
      projectHost,
      asString(req.params.jobId, "jobId"),
      knownJobs,
    );
    if (job.status.status !== "completed" || !job.output) {
      throw new HttpError(409, "render output is not complete");
    }
    const output = path.resolve(job.output);
    if (!isContained(projectHost.dir, output)) {
      throw new HttpError(500, "render output escaped project directory");
    }
    try {
      const details = await stat(output);
      if (!details.isFile()) throw new Error("not a regular file");
    } catch {
      throw new HttpError(404, "render output is unavailable");
    }
    await sendFile(res, output);
  }));

  app.post("/api/agent/reset", withErrors(async (_req, res) => {
    await resetAgentSession();
    res.json({ ok: true });
  }));

  app.get("/api/agent/config", withErrors(async (_req, res) => {
    res.json(agentConfigResponse(agentConfig));
  }));

  app.post("/api/agent/config", withErrors(async (req, res) => {
    const body = asRecord(req.body === undefined ? {} : req.body, "request body");
    res.json({ ok: true, config: await updateAgentConfig(body) });
  }));

  app.post("/api/agent", withErrors(async (req, res) => {
    const body = asRecord(req.body, "request body");
    const prompt = asString(body.prompt, "prompt");
    const ndjson = acceptNdjson(req);
    const events: AgentEvent[] = [];
    if (ndjson) {
      res.setHeader("Content-Type", "application/x-ndjson; charset=utf-8");
      res.setHeader("Cache-Control", "no-cache");
      res.flushHeaders?.();
    }
    let activeSession: AgentSession | undefined;
    let finished = false;
    req.on("close", () => {
      if (!finished) activeSession?.abort();
    });
    const result = runAgent(prompt, (event) => {
      events.push(event);
      if (ndjson && !res.writableEnded) res.write(JSON.stringify(event) + "\n");
    });
    void getSession().then((session) => {
      activeSession = session;
    }, () => undefined);
    try {
      const response = await result;
      finished = true;
      const currentState = await state();
      if (ndjson) {
        res.write(JSON.stringify({ type: "result", text: response.text, state: currentState }) + "\n");
        res.end();
      } else {
        res.json({ text: response.text, events, state: currentState });
      }
    } catch (error) {
      finished = true;
      if (ndjson) {
        if (!res.writableEnded) {
          res.write(JSON.stringify({ type: "error", ...jsonError(error) }) + "\n");
          res.end();
        }
      } else {
        sendError(res, error);
      }
    }
  }));

  const distribution = guiDistPath(options);
  if (distribution) {
    app.use(express.static(distribution));
    const index = path.join(distribution, "index.html");
    app.use((req: RequestLike, res: ResponseLike, next: NextLike) => {
      if (req.method !== "GET" || res.headersSent || (
        typeof req.headers.accept === "string" && req.headers.accept.includes("application/json")
      )) {
        next();
        return;
      }
      res.sendFile(index, { acceptRanges: false }, (error: Error | undefined) => {
        if (error) next(error);
      });
    });
  }

  app.use((error: unknown, _req: RequestLike, res: ResponseLike, _next: NextLike) => {
    if (!res.headersSent) sendError(res, error);
  });

  if (options.media !== undefined) {
    try {
      await importMedia(options.media);
    } catch (error) {
      await projectHost.close();
      runtime.close();
      throw error;
    }
  }

  const close = async (): Promise<void> => {
    if (closed) return;
    closed = true;
    await projectHost.close();
    runtime.close();
  };
  return { app, host: projectHost, importMedia, close };
}

/** Create and listen on the local HTTP application. */
export async function startCutApp(options: CutAppOptions): Promise<StartedCutApp> {
  const created = await createCutApp(options);
  const bindHost = options.host ?? DEFAULT_HOST;
  const requestedPort = options.port ?? DEFAULT_PORT;
  let server: Server;
  try {
    server = await new Promise<Server>((resolve, reject) => {
      const candidate = created.app.listen(requestedPort, bindHost, () => resolve(candidate));
      candidate.once("error", reject);
    });
  } catch (error) {
    await created.close();
    throw error;
  }
  const address = server.address();
  const port = typeof address === "object" && address !== null ? address.port : requestedPort;
  const url = "http://" + bindHost + ":" + port;
  let closed = false;
  const close = async (): Promise<void> => {
    if (closed) return;
    closed = true;
    await new Promise<void>((resolve, reject) => {
      server.close((error) => error ? reject(error) : resolve());
    });
    await created.close();
  };
  const result: StartedCutApp = {
    app: created.app,
    importMedia: created.importMedia,
    close,
    url,
    port,
    host: bindHost,
    projectHost: created.host,
    server,
  };
  if (options.openBrowser) openBrowser(url);
  return result;
}

export type { AgentSessionOptions };
