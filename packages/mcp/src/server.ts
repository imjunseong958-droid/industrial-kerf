import {
  CallToolRequestSchema,
  ListToolsRequestSchema,
  type CallToolResult,
} from "@modelcontextprotocol/sdk/types.js";
import { Server } from "@modelcontextprotocol/sdk/server/index.js";
import type { Transport } from "@modelcontextprotocol/sdk/shared/transport.js";

/**
 * JSON Schema for a cut capability. The server keeps this structural instead of
 * importing the timeline package so a host can supply its own timeline and still
 * expose the same MCP contract.
 */
export type CutMcpJsonSchema = {
  [key: string]: unknown;
};

export type CutMcpObjectSchema = CutMcpJsonSchema & {
  type: "object";
};

export type CutMcpToolAnnotations = {
  title?: string;
  readOnlyHint?: boolean;
  destructiveHint?: boolean;
  idempotentHint?: boolean;
  openWorldHint?: boolean;
};

/** A tool definition returned by tools/list. */
export interface CutMcpToolDefinition {
  name: string;
  description: string;
  inputSchema: CutMcpObjectSchema;
  annotations?: CutMcpToolAnnotations;
}

/** Base (un-namespaced) tool names offered by this capability layer. */
export const CUT_MCP_TOOL_NAMES = [
  "project.inspect",
  "timeline.query",
  "timeline.apply",
  "timeline.undo",
  "timeline.redo",
  "project.validate",
  "timeline.diff",
  "render.preview",
  "render.start",
  "render.status",
  "render.cancel",
  "render.result",
  "media.fetch",
  "media.import",
  "media.probe",
  "vision.track_objects",
  "skills.read",
] as const;

export type CutMcpToolName = (typeof CUT_MCP_TOOL_NAMES)[number];

export type CutMcpProfile = "core" | "full";

/**
 * Core editing loop advertised by cut-mcp unless `--full` / CUT_MCP_PROFILE=full
 * is selected. Kept as CUT_MCP_MVP_TOOL_NAMES for compatibility.
 */
export const CUT_MCP_MVP_TOOL_NAMES = [
  "timeline.query",
  "timeline.apply",
  "timeline.undo",
  "timeline.redo",
  "media.probe",
  "media.fetch",
  "media.import",
  "render.preview",
  "render.start",
  "render.result",
  "skills.read",
] as const satisfies readonly CutMcpToolName[];

export const CUT_MCP_CORE_TOOL_NAMES = CUT_MCP_MVP_TOOL_NAMES;

export const CUT_MCP_FULL_TOOL_NAMES = CUT_MCP_TOOL_NAMES;

export type CutMcpMvpToolName = (typeof CUT_MCP_MVP_TOOL_NAMES)[number];
export type CutMcpCoreToolName = CutMcpMvpToolName;

/** Codes used in structured tool errors. */
export const CUT_MCP_ERROR_CODES = {
  unknownTool: "UNKNOWN_TOOL",
  invalidArguments: "INVALID_ARGUMENTS",
  capabilityUnavailable: "CAPABILITY_UNAVAILABLE",
  handlerFailed: "HANDLER_FAILED",
  runtimeError: "RUNTIME_ERROR",
  revisionConflict: "REVISION_CONFLICT",
  requestAborted: "REQUEST_ABORTED",
  serializationFailed: "SERIALIZATION_FAILED",
} as const;

export type CutMcpErrorCode =
  (typeof CUT_MCP_ERROR_CODES)[keyof typeof CUT_MCP_ERROR_CODES];

export interface CutMcpErrorShape {
  code: CutMcpErrorCode;
  message: string;
  details?: unknown;
}

/** Error type a host may throw to preserve a stable structured error code. */
export class CutMcpError extends Error {
  readonly code: CutMcpErrorCode;
  readonly details: unknown;

  constructor(code: CutMcpErrorCode, message: string, details?: unknown) {
    super(message);
    this.name = "CutMcpError";
    this.code = code;
    this.details = details;
  }
}

export interface CutMcpSuccess {
  ok: true;
  result: unknown;
}

export interface CutMcpFailure {
  ok: false;
  error: CutMcpErrorShape;
}

export type CutMcpStructuredResult = CutMcpSuccess | CutMcpFailure;

/** MCP's JSON result with a predictable structuredContent envelope. */
export interface CutMcpCallResult {
  content: [{ type: "text"; text: string }];
  structuredContent: Record<string, unknown>;
  isError?: boolean;
}

export interface CutMcpRequestContext {
  /** The namespaced name presented to the caller. */
  toolName: string;
  /** The base name used by the capability registry. */
  capability: CutMcpToolName;
  signal?: AbortSignal;
  requestId?: string | number;
  sessionId?: string;
}

export type CutMcpHandler<I = unknown, O = unknown> = (
  input: I,
  context: Readonly<CutMcpRequestContext>,
) => O | Promise<O>;

// These input interfaces intentionally describe only the stable protocol.
// Handler output is host-defined, so it remains unknown here.
export interface ProjectInspectInput {
  include?: "summary" | "full";
}

export interface ProjectValidateInput {
  strict?: boolean;
}

export interface TimelineQueryInput {
  include?: "summary" | "full";
  trackId?: string;
  clipId?: string;
}

export interface TimelineApplyInput {
  expectedRevision?: number;
  idempotencyKey?: string;
  op?: Record<string, unknown>;
  ops?: Array<Record<string, unknown>>;
  /** One canonical v2 project command. */
  command?: Record<string, unknown>;
  /** A non-empty atomic v2 command batch. */
  commands?: Array<Record<string, unknown>>;
}

export interface TimelineHistoryInput {
  steps?: number;
  expectedRevision?: number;
  idempotencyKey?: string;
}

export interface TimelineDiffInput {
  from?: Record<string, unknown>;
  to?: Record<string, unknown>;
}

export interface RenderPreviewInput {
  draft?: boolean;
}

export interface RenderStartInput {
  draft?: boolean;
}

export interface RenderJobInput {
  jobId: string;
}

export interface MediaFetchInput {
  url: string;
  cookies?: string;
  idempotencyKey?: string;
}

export interface MediaImportInput {
  path: string;
  idempotencyKey?: string;
  metadata?: Record<string, unknown>;
  overlay?: Record<string, unknown>;
}

export interface MediaProbeInput {
  action: "info" | "silence" | "scenes" | "frames";
  src: string;
  opts?: {
    frames?: number;
    noiseDb?: number;
    minDuration?: number;
  };
}

export interface VisionTrackObjectsInput {
  source: string;
  fps?: number;
  frameWidth?: number;
  frameHeight?: number;
  maxFrames?: number;
  crop?: {
    aspectRatio: number;
    padding?: number;
  };
}

export interface SkillsReadInput {
  name: string;
}

export interface CutMcpProjectHandlers {
  inspect?: CutMcpHandler<ProjectInspectInput>;
  validate?: CutMcpHandler<ProjectValidateInput>;
}

export interface CutMcpTimelineHandlers {
  query?: CutMcpHandler<TimelineQueryInput>;
  apply?: CutMcpHandler<TimelineApplyInput>;
  undo?: CutMcpHandler<TimelineHistoryInput>;
  redo?: CutMcpHandler<TimelineHistoryInput>;
  diff?: CutMcpHandler<TimelineDiffInput>;
}

export interface CutMcpRenderHandlers {
  preview?: CutMcpHandler<RenderPreviewInput>;
  start?: CutMcpHandler<RenderStartInput>;
  status?: CutMcpHandler<RenderJobInput>;
  cancel?: CutMcpHandler<RenderJobInput>;
  result?: CutMcpHandler<RenderJobInput>;
}

export interface CutMcpMediaHandlers {
  fetch?: CutMcpHandler<MediaFetchInput>;
  import?: CutMcpHandler<MediaImportInput>;
  probe?: CutMcpHandler<MediaProbeInput>;
}

export interface CutMcpVisionHandlers {
  track_objects?: CutMcpHandler<VisionTrackObjectsInput>;
}

export interface CutMcpSkillsHandlers {
  read?: CutMcpHandler<SkillsReadInput>;
}

/**
 * Injectable capability implementations.  Nested groups are the preferred
 * form.  Flat dotted keys are accepted too, which is handy when capabilities
 * are assembled by a plugin registry.
 */
export type CutMcpHandlers = {
  project?: CutMcpProjectHandlers;
  timeline?: CutMcpTimelineHandlers;
  render?: CutMcpRenderHandlers;
  media?: CutMcpMediaHandlers;
  vision?: CutMcpVisionHandlers;
  skills?: CutMcpSkillsHandlers;
} & Partial<Record<CutMcpToolName, CutMcpHandler>>;

export interface CutMcpRouterOptions {
  /**
   * Optional server namespace.  With `namespace: "cut"`, for example,
   * `timeline.query` is advertised as `cut.timeline.query`.  Dotted base
   * names remain the default so they work with MCP clients that already use
   * a server-level namespace.
   */
  namespace?: string;
  /**
   * Advertise capabilities that do not have an injected implementation.  The
   * default is false: a host only advertises callable capabilities.  When
   * enabled, calls return a structured CAPABILITY_UNAVAILABLE error.
   */
  includeUnavailable?: boolean;
  /**
   * Optional advertisement filter.  The default is unfiltered (every injected
   * handler, equivalent to `"full"`) so unit tests and `createCutAgentTools`
   * keep seeing inspect/validate/diff/status/cancel/vision.  `"core"` limits
   * the router to {@link CUT_MCP_CORE_TOOL_NAMES}.
   */
  profile?: CutMcpProfile;
}

export interface CutMcpServerOptions extends CutMcpRouterOptions {
  name?: string;
  version?: string;
  instructions?: string;
}

interface CutMcpToolEntry {
  baseName: CutMcpToolName;
  name: string;
  definition: CutMcpToolDefinition;
  handler?: CutMcpHandler;
}

export interface CutMcpSchemaIssue {
  path: string;
  message: string;
}

const enumValues = (values: readonly unknown[]): CutMcpJsonSchema => ({
  enum: [...values],
});

const operationSchema: CutMcpJsonSchema = {
  type: "object",
  description: "One validated timeline operation.",
  properties: {
    op: {
      type: "string",
      enum: [
        "trim",
        "split",
        "remove_ranges",
        "set_speed",
        "set_volume",
        "add_subtitles",
        "remove_overlay",
        "add_source",
        "add_clip",
        "remove_clip",
        "move_clip",
        "set_crop",
        "add_captions",
      ],
    },
  },
  required: ["op"],
  // Operation-specific fields intentionally stay open for forward-compatible
  // timeline schema additions.  The operation discriminator is constrained.
  additionalProperties: true,
};

/**
 * Canonical v2 command discriminators.  Payload leaves are intentionally
 * structural here because the host validates the complete command with
 * ProjectCommandSchema; the discriminator and required command envelope are
 * still closed so arbitrary operations cannot enter through MCP.
 */
const projectCommandSchema: CutMcpJsonSchema = {
  type: "object",
  oneOf: [
    {
      type: "object",
      properties: { type: { const: "add_track" }, track: { type: "object" } },
      required: ["type", "track"],
      additionalProperties: true,
    },
    {
      type: "object",
      properties: { type: { const: "remove_track" }, trackId: { type: "string", minLength: 1 } },
      required: ["type", "trackId"],
      additionalProperties: true,
    },
    {
      type: "object",
      properties: {
        type: { const: "add_clip" },
        trackId: { type: "string", minLength: 1 },
        clip: { type: "object" },
        index: { type: "integer", minimum: 0 },
      },
      required: ["type", "trackId", "clip"],
      additionalProperties: true,
    },
    {
      type: "object",
      properties: {
        type: { const: "remove_clip" },
        clipId: { type: "string", minLength: 1 },
        trackId: { type: "string", minLength: 1 },
      },
      required: ["type", "clipId"],
      additionalProperties: true,
    },
    {
      type: "object",
      properties: {
        type: { const: "move_clip" },
        clipId: { type: "string", minLength: 1 },
        trackId: { type: "string", minLength: 1 },
        toTrackId: { type: "string", minLength: 1 },
        index: { type: "integer", minimum: 0 },
        toIndex: { type: "integer", minimum: 0 },
      },
      required: ["type", "clipId"],
      oneOf: [{ required: ["index"] }, { required: ["toIndex"] }],
      additionalProperties: true,
    },
    {
      type: "object",
      properties: {
        type: { const: "update_clip" },
        clipId: { type: "string", minLength: 1 },
        patch: { type: "object" },
      },
      required: ["type", "clipId", "patch"],
      additionalProperties: true,
    },
    {
      type: "object",
      properties: {
        type: { const: "set_clip" },
        clipId: { type: "string", minLength: 1 },
        clip: { type: "object" },
      },
      required: ["type", "clipId", "clip"],
      additionalProperties: true,
    },
    {
      type: "object",
      properties: { type: { const: "add_marker" }, marker: { type: "object" } },
      required: ["type", "marker"],
      additionalProperties: true,
    },
    {
      type: "object",
      properties: { type: { const: "remove_marker" }, markerId: { type: "string", minLength: 1 } },
      required: ["type", "markerId"],
      additionalProperties: true,
    },
    {
      type: "object",
      properties: { type: { const: "set_metadata" }, patch: { type: "object" } },
      required: ["type", "patch"],
      additionalProperties: true,
    },
    {
      type: "object",
      properties: { type: { const: "add_source" }, source: { type: "object" } },
      required: ["type", "source"],
      additionalProperties: true,
    },
    {
      type: "object",
      properties: { type: { const: "add_overlay" }, overlay: { type: "object" } },
      required: ["type", "overlay"],
      additionalProperties: true,
    },
    {
      type: "object",
      properties: { type: { const: "remove_overlay" }, overlayId: { type: "string", minLength: 1 } },
      required: ["type", "overlayId"],
      additionalProperties: true,
    },
    {
      type: "object",
      properties: {
        type: { const: "batch" },
        commands: {
          type: "array",
          minItems: 1,
          items: {
            type: "object",
            properties: {
              type: {
                type: "string",
                enum: [
                  "add_track",
                  "remove_track",
                  "add_clip",
                  "remove_clip",
                  "move_clip",
                  "update_clip",
                  "set_clip",
                  "add_marker",
                  "remove_marker",
                  "set_metadata",
                  "add_source",
                  "add_overlay",
                  "remove_overlay",
                  "batch",
                ],
              },
            },
            required: ["type"],
            additionalProperties: true,
          },
        },
      },
      required: ["type", "commands"],
      additionalProperties: true,
    },
  ],
  additionalProperties: true,
};

const expectedRevisionSchema: CutMcpJsonSchema = {
  type: "integer",
  minimum: 0,
  description: "Only apply the request if the project is still at this revision.",
};

const exactlyOneApplyPayload: CutMcpJsonSchema = {
  oneOf: [
    {
      required: ["op"],
      not: {
        anyOf: [
          { required: ["ops"] },
          { required: ["command"] },
          { required: ["commands"] },
        ],
      },
    },
    {
      required: ["ops"],
      not: {
        anyOf: [
          { required: ["op"] },
          { required: ["command"] },
          { required: ["commands"] },
        ],
      },
    },
    {
      required: ["command"],
      not: {
        anyOf: [
          { required: ["op"] },
          { required: ["ops"] },
          { required: ["commands"] },
        ],
      },
    },
    {
      required: ["commands"],
      not: {
        anyOf: [
          { required: ["op"] },
          { required: ["ops"] },
          { required: ["command"] },
        ],
      },
    },
  ],
};

const toolDefinitions: Record<CutMcpToolName, Omit<CutMcpToolDefinition, "name">> = {
  "project.inspect": {
    description: "Inspect the current project and its validated timeline metadata.",
    inputSchema: {
      type: "object",
      properties: { include: enumValues(["summary", "full"]) },
      additionalProperties: false,
    },
    annotations: { readOnlyHint: true, idempotentHint: true },
  },
  "timeline.query": {
    description: "Query the current validated timeline, sources, tracks, and clips.",
    inputSchema: {
      type: "object",
      properties: {
        include: enumValues(["summary", "full"]),
        trackId: { type: "string", minLength: 1 },
        clipId: { type: "string", minLength: 1 },
      },
      additionalProperties: false,
    },
    annotations: { readOnlyHint: true, idempotentHint: true },
  },
  "timeline.apply": {
    description: "Apply one operation or a non-empty atomic batch to the timeline.",
    inputSchema: {
      type: "object",
      properties: {
        expectedRevision: expectedRevisionSchema,
        idempotencyKey: { type: "string", minLength: 1 },
        op: operationSchema,
        ops: { type: "array", minItems: 1, items: operationSchema },
        command: projectCommandSchema,
        commands: { type: "array", minItems: 1, items: projectCommandSchema },
      },
      ...exactlyOneApplyPayload,
      additionalProperties: false,
    },
    annotations: { destructiveHint: true, idempotentHint: false },
  },
  "timeline.undo": {
    description: "Undo one or more previously applied timeline transactions.",
    inputSchema: {
      type: "object",
      properties: {
        steps: { type: "integer", minimum: 1, maximum: 100 },
        expectedRevision: expectedRevisionSchema,
        idempotencyKey: { type: "string", minLength: 1 },
      },
      additionalProperties: false,
    },
    annotations: { destructiveHint: true, idempotentHint: false },
  },
  "timeline.redo": {
    description: "Redo one or more previously undone timeline transactions.",
    inputSchema: {
      type: "object",
      properties: {
        steps: { type: "integer", minimum: 1, maximum: 100 },
        expectedRevision: expectedRevisionSchema,
        idempotencyKey: { type: "string", minLength: 1 },
      },
      additionalProperties: false,
    },
    annotations: { destructiveHint: true, idempotentHint: false },
  },
  "project.validate": {
    description: "Validate the current project state and return structured diagnostics.",
    inputSchema: {
      type: "object",
      properties: { strict: { type: "boolean" } },
      additionalProperties: false,
    },
    annotations: { readOnlyHint: true, idempotentHint: true },
  },
  "timeline.diff": {
    description: "Compare two host-supplied timeline snapshots or the current state.",
    inputSchema: {
      type: "object",
      properties: {
        from: { type: "object" },
        to: { type: "object" },
      },
      additionalProperties: false,
    },
    annotations: { readOnlyHint: true, idempotentHint: true },
  },
  "render.preview": {
    description: "Create a fast preview render for the current timeline.",
    inputSchema: {
      type: "object",
      properties: { draft: { type: "boolean" } },
      additionalProperties: false,
    },
    annotations: { idempotentHint: true },
  },
  "render.start": {
    description: "Start a render job for the current validated timeline.",
    inputSchema: {
      type: "object",
      properties: { draft: { type: "boolean" } },
      additionalProperties: false,
    },
    annotations: { destructiveHint: true, idempotentHint: false },
  },
  "render.status": {
    description: "Read the status of a render job.",
    inputSchema: {
      type: "object",
      properties: { jobId: { type: "string", minLength: 1 } },
      required: ["jobId"],
      additionalProperties: false,
    },
    annotations: { readOnlyHint: true, idempotentHint: true },
  },
  "render.cancel": {
    description: "Cancel a running render job.",
    inputSchema: {
      type: "object",
      properties: { jobId: { type: "string", minLength: 1 } },
      required: ["jobId"],
      additionalProperties: false,
    },
    annotations: { destructiveHint: true, idempotentHint: true },
  },
  "render.result": {
    description: "Retrieve the result of a completed render job.",
    inputSchema: {
      type: "object",
      properties: { jobId: { type: "string", minLength: 1 } },
      required: ["jobId"],
      additionalProperties: false,
    },
    annotations: { readOnlyHint: true, idempotentHint: true },
  },
  "media.fetch": {
    description: "Download media from a URL and append it to the project timeline.",
    inputSchema: {
      type: "object",
      properties: {
        url: { type: "string", minLength: 1, description: "Video/media URL to fetch." },
        cookies: { type: "string", description: "Optional cookie file path for authenticated downloads." },
        idempotencyKey: { type: "string", minLength: 1 },
      },
      required: ["url"],
      additionalProperties: false,
    },
    annotations: { destructiveHint: true, idempotentHint: false },
  },
  "media.import": {
    description: "Import a local media file and append it to the project timeline.",
    inputSchema: {
      type: "object",
      properties: {
        path: { type: "string", minLength: 1, description: "Local media path relative to the project directory." },
        idempotencyKey: { type: "string", minLength: 1 },
        metadata: { type: "object" },
        overlay: { type: "object" },
      },
      required: ["path"],
      additionalProperties: false,
    },
    annotations: { destructiveHint: true, idempotentHint: false },
  },
  "media.probe": {
    description:
      "Inspect media. The frames action extracts PNGs for visual inspection; use returned paths and metadata for crop planning.",
    inputSchema: {
      type: "object",
      properties: {
        action: { type: "string", enum: ["info", "silence", "scenes", "frames"] },
        src: { type: "string", minLength: 1, description: "Local media path." },
        opts: {
          type: "object",
          properties: {
            frames: {
              type: "integer",
              minimum: 1,
              maximum: 8,
              description: "Number of PNG frames to extract (maximum 8).",
            },
            noiseDb: { type: "number" },
            minDuration: { type: "number", minimum: 0 },
          },
          additionalProperties: false,
        },
      },
      required: ["action", "src"],
      additionalProperties: false,
    },
    annotations: { readOnlyHint: true, idempotentHint: true },
  },
  "vision.track_objects": {
    description:
      "Track configured cam (streamer camera), chat (chat overlay), and content (main gameplay/stream canvas) objects and return class-separated ROI/crop planning data. " +
      "Use plan.keyframesByClass.content when framing the main picture. Weights and labels come only from project/user configuration; they are not selectable in tool input.",
    inputSchema: {
      type: "object",
      description:
        "Track configured cam, chat, and content objects. content is the main gameplay/stream canvas. Model weights and labels are fixed by project/user vision configuration.",
      properties: {
        source: { type: "string", minLength: 1, description: "Local media path." },
        fps: { type: "number", exclusiveMinimum: 0 },
        frameWidth: { type: "number", exclusiveMinimum: 0 },
        frameHeight: { type: "number", exclusiveMinimum: 0 },
        maxFrames: { type: "integer", minimum: 1 },
        crop: {
          type: "object",
          properties: {
            aspectRatio: { type: "number", exclusiveMinimum: 0 },
            padding: { type: "number", minimum: 0 },
          },
          required: ["aspectRatio"],
          additionalProperties: false,
        },
      },
      required: ["source"],
      additionalProperties: false,
    },
    annotations: { readOnlyHint: true, idempotentHint: true },
  },
  "skills.read": {
    description: "Read a loaded playbook body by name.",
    inputSchema: {
      type: "object",
      properties: { name: { type: "string", minLength: 1 } },
      required: ["name"],
      additionalProperties: false,
    },
    annotations: { readOnlyHint: true, idempotentHint: true },
  },
};

/** JSON Schemas exposed by the capability router, keyed by base tool name. */
export const CUT_MCP_TOOL_SCHEMAS: Readonly<
  Record<CutMcpToolName, CutMcpObjectSchema>
> = Object.freeze(
  Object.fromEntries(
    CUT_MCP_TOOL_NAMES.map((name) => [name, toolDefinitions[name].inputSchema]),
  ) as Record<CutMcpToolName, CutMcpObjectSchema>,
);

export const CUT_MCP_SCHEMAS = CUT_MCP_TOOL_SCHEMAS;

function normalizeNamespace(namespace: string | undefined): string {
  if (namespace === undefined) return "";
  return namespace.trim().replace(/\.+$/, "");
}

/** Add an optional server namespace to one of the dotted capability names. */
export function namespaceCutMcpToolName(
  namespace: string | undefined,
  capability: CutMcpToolName,
): string {
  const normalized = normalizeNamespace(namespace);
  return normalized ? `${normalized}.${capability}` : capability;
}

/** Remove an optional namespace and return the known base capability name. */
export function parseCutMcpToolName(
  name: string,
  namespace?: string,
): CutMcpToolName | undefined {
  const normalized = normalizeNamespace(namespace);
  const base = normalized
    ? name.startsWith(`${normalized}.`)
      ? name.slice(normalized.length + 1)
      : undefined
    : name;
  return base !== undefined && (CUT_MCP_TOOL_NAMES as readonly string[]).includes(base)
    ? (base as CutMcpToolName)
    : undefined;
}

function groupAndMethod(capability: CutMcpToolName): [string, string] {
  const dot = capability.indexOf(".");
  return [capability.slice(0, dot), capability.slice(dot + 1)];
}

function findHandler(
  handlers: CutMcpHandlers,
  capability: CutMcpToolName,
): CutMcpHandler | undefined {
  const flat = handlers as Record<string, unknown>;
  const direct = flat[capability];
  if (typeof direct === "function") return direct as CutMcpHandler;

  const [groupName, methodName] = groupAndMethod(capability);
  const group = flat[groupName];
  if (!group || typeof group !== "object") return undefined;
  const nested = (group as Record<string, unknown>)[methodName];
  return typeof nested === "function" ? (nested as CutMcpHandler) : undefined;
}

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function pathForKey(path: string, key: string): string {
  return /^[A-Za-z_$][A-Za-z0-9_$]*$/.test(key) ? `${path}.${key}` : `${path}[${JSON.stringify(key)}]`;
}

function sameJsonValue(left: unknown, right: unknown): boolean {
  if (Object.is(left, right)) return true;
  try {
    return JSON.stringify(left) === JSON.stringify(right);
  } catch {
    return false;
  }
}

function schemaTypeMatches(value: unknown, type: string): boolean {
  switch (type) {
    case "object":
      return isObject(value);
    case "array":
      return Array.isArray(value);
    case "string":
      return typeof value === "string";
    case "number":
      return typeof value === "number" && Number.isFinite(value);
    case "integer":
      return typeof value === "number" && Number.isInteger(value);
    case "boolean":
      return typeof value === "boolean";
    case "null":
      return value === null;
    default:
      return true;
  }
}

function validateSchema(
  schema: unknown,
  value: unknown,
  path: string,
  issues: CutMcpSchemaIssue[],
): boolean {
  if (!isObject(schema)) return true;

  const start = issues.length;
  const type = schema.type;
  if (typeof type === "string") {
    if (!schemaTypeMatches(value, type)) {
      issues.push({ path, message: `must be ${type}` });
      return false;
    }
  } else if (Array.isArray(type)) {
    if (!type.some((candidate) => typeof candidate === "string" && schemaTypeMatches(value, candidate))) {
      issues.push({ path, message: `must be one of ${type.join(", ")}` });
      return false;
    }
  }

  if ("const" in schema && !sameJsonValue(value, schema.const)) {
    issues.push({ path, message: `must equal ${JSON.stringify(schema.const)}` });
  }
  if (Array.isArray(schema.enum) && !schema.enum.some((candidate) => sameJsonValue(value, candidate))) {
    issues.push({ path, message: `must be one of ${schema.enum.map(String).join(", ")}` });
  }

  if (Array.isArray(schema.oneOf)) {
    let matches = 0;
    for (const branch of schema.oneOf) {
      const branchIssues: CutMcpSchemaIssue[] = [];
      if (validateSchema(branch, value, path, branchIssues) && branchIssues.length === 0) matches += 1;
    }
    if (matches !== 1) {
      issues.push({ path, message: matches === 0 ? "must match one allowed shape" : "must match only one allowed shape" });
    }
  }
  if (Array.isArray(schema.anyOf)) {
    const matches = schema.anyOf.some((branch) => {
      const branchIssues: CutMcpSchemaIssue[] = [];
      return validateSchema(branch, value, path, branchIssues) && branchIssues.length === 0;
    });
    if (!matches) issues.push({ path, message: "must match one allowed shape" });
  }
  if ("not" in schema) {
    const notIssues: CutMcpSchemaIssue[] = [];
    if (validateSchema(schema.not, value, path, notIssues) && notIssues.length === 0) {
      issues.push({ path, message: "must not match the disallowed shape" });
    }
  }

  if (typeof value === "string") {
    if (typeof schema.minLength === "number" && value.length < schema.minLength) {
      issues.push({ path, message: `must contain at least ${schema.minLength} characters` });
    }
    if (typeof schema.maxLength === "number" && value.length > schema.maxLength) {
      issues.push({ path, message: `must contain at most ${schema.maxLength} characters` });
    }
    if (typeof schema.pattern === "string") {
      let matches = false;
      try {
        matches = new RegExp(schema.pattern).test(value);
      } catch {
        matches = true;
      }
      if (!matches) issues.push({ path, message: "has an invalid format" });
    }
  }

  if (typeof value === "number") {
    if (typeof schema.minimum === "number" && value < schema.minimum) {
      issues.push({ path, message: `must be >= ${schema.minimum}` });
    }
    if (typeof schema.maximum === "number" && value > schema.maximum) {
      issues.push({ path, message: `must be <= ${schema.maximum}` });
    }
    if (typeof schema.exclusiveMinimum === "number" && value <= schema.exclusiveMinimum) {
      issues.push({ path, message: `must be > ${schema.exclusiveMinimum}` });
    }
    if (typeof schema.exclusiveMaximum === "number" && value >= schema.exclusiveMaximum) {
      issues.push({ path, message: `must be < ${schema.exclusiveMaximum}` });
    }
  }

  if (Array.isArray(value)) {
    if (typeof schema.minItems === "number" && value.length < schema.minItems) {
      issues.push({ path, message: `must contain at least ${schema.minItems} items` });
    }
    if (typeof schema.maxItems === "number" && value.length > schema.maxItems) {
      issues.push({ path, message: `must contain at most ${schema.maxItems} items` });
    }
    if (schema.items !== undefined) {
      value.forEach((item, index) => validateSchema(schema.items, item, `${path}[${index}]`, issues));
    }
  }

  if (isObject(value)) {
    const properties = isObject(schema.properties) ? schema.properties : {};
    if (Array.isArray(schema.required)) {
      for (const required of schema.required) {
        if (typeof required === "string" && !Object.prototype.hasOwnProperty.call(value, required)) {
          issues.push({ path, message: `must include ${JSON.stringify(required)}` });
        }
      }
    }
    for (const [key, propertyValue] of Object.entries(value)) {
      const propertySchema = properties[key];
      if (propertySchema !== undefined) {
        validateSchema(propertySchema, propertyValue, pathForKey(path, key), issues);
      } else if (schema.additionalProperties === false) {
        issues.push({ path: pathForKey(path, key), message: "is not allowed" });
      } else if (isObject(schema.additionalProperties)) {
        validateSchema(schema.additionalProperties, propertyValue, pathForKey(path, key), issues);
      }
    }
  }

  return issues.length === start;
}

/** Validate a JSON value against one of the advertised capability schemas. */
export function validateCutMcpArguments(
  schema: CutMcpJsonSchema,
  value: unknown,
): CutMcpSchemaIssue[] {
  const issues: CutMcpSchemaIssue[] = [];
  validateSchema(schema, value, "$", issues);
  return issues;
}

function errorShape(error: CutMcpError): CutMcpErrorShape {
  return error.details === undefined
    ? { code: error.code, message: error.message }
    : { code: error.code, message: error.message, details: error.details };
}

function asError(error: unknown): CutMcpError {
  if (error instanceof CutMcpError) return error;
  if (error instanceof Error) {
    return new CutMcpError(CUT_MCP_ERROR_CODES.handlerFailed, error.message);
  }
  return new CutMcpError(CUT_MCP_ERROR_CODES.handlerFailed, String(error));
}

function jsonText(value: unknown): string {
  const serialized = JSON.stringify(value);
  return serialized === undefined ? "null" : serialized;
}

function resultForStructured(structured: CutMcpStructuredResult): CutMcpCallResult {
  const content: [{ type: "text"; text: string }] = [
    { type: "text", text: jsonText(structured) },
  ];
  return structured.ok
    ? { content, structuredContent: structured as unknown as Record<string, unknown> }
    : {
        content,
        structuredContent: structured as unknown as Record<string, unknown>,
        isError: true,
      };
}

function unavailableHandler(capability: CutMcpToolName): CutMcpHandler {
  return () => {
    throw new CutMcpError(
      CUT_MCP_ERROR_CODES.capabilityUnavailable,
      `Capability ${capability} has no injected handler`,
    );
  };
}

const CORE_TOOL_NAME_SET = new Set<string>(CUT_MCP_CORE_TOOL_NAMES);

function advertisedToolNames(profile: CutMcpProfile | undefined): readonly CutMcpToolName[] {
  if (profile === "core") {
    return CUT_MCP_TOOL_NAMES.filter((name) => CORE_TOOL_NAME_SET.has(name));
  }
  return CUT_MCP_TOOL_NAMES;
}

/**
 * Pure capability router. It is independent from a transport and is the
 * recommended injected handler group for unit tests and CLI/project wiring.
 */
export class CutMcpRouter {
  private readonly entriesByName = new Map<string, CutMcpToolEntry>();

  readonly namespace: string;

  constructor(
    handlers: CutMcpHandlers = {},
    options: CutMcpRouterOptions = {},
  ) {
    this.namespace = normalizeNamespace(options.namespace);
    const includeUnavailable = options.includeUnavailable ?? false;
    for (const baseName of advertisedToolNames(options.profile)) {
      const handler = findHandler(handlers, baseName);
      if (!handler && !includeUnavailable) continue;
      const name = namespaceCutMcpToolName(this.namespace, baseName);
      this.entriesByName.set(name, {
        baseName,
        name,
        definition: { name, ...toolDefinitions[baseName] },
        ...(handler ? { handler } : { handler: unavailableHandler(baseName) }),
      });
    }
  }

  /** Tool definitions suitable for an MCP tools/list response. */
  listTools(): CutMcpToolDefinition[] {
    return [...this.entriesByName.values()].map((entry) => ({
      ...entry.definition,
      inputSchema: { ...entry.definition.inputSchema },
      ...(entry.definition.annotations
        ? { annotations: { ...entry.definition.annotations } }
        : {}),
    }));
  }

  get toolNames(): string[] {
    return [...this.entriesByName.keys()];
  }

  hasTool(name: string): boolean {
    return this.entriesByName.has(name);
  }

  /** Invoke a capability and always return an MCP-safe structured result. */
  async call(
    name: string,
    input: unknown = {},
    context?: Partial<Omit<CutMcpRequestContext, "toolName" | "capability">>,
  ): Promise<CutMcpCallResult> {
    const entry = this.entriesByName.get(name);
    if (!entry) {
      return resultForStructured({
        ok: false,
        error: {
          code: CUT_MCP_ERROR_CODES.unknownTool,
          message: `Unknown cut MCP tool: ${name}`,
        },
      });
    }

    const actualInput = input === undefined ? {} : input;
    const issues = validateCutMcpArguments(entry.definition.inputSchema, actualInput);
    if (issues.length > 0) {
      return resultForStructured({
        ok: false,
        error: {
          code: CUT_MCP_ERROR_CODES.invalidArguments,
          message: `Invalid arguments for ${name}`,
          details: { issues },
        },
      });
    }

    const invocationContext: CutMcpRequestContext = {
      toolName: entry.name,
      capability: entry.baseName,
      ...(context?.signal ? { signal: context.signal } : {}),
      ...(context?.requestId !== undefined ? { requestId: context.requestId } : {}),
      ...(context?.sessionId !== undefined ? { sessionId: context.sessionId } : {}),
    };

    if (invocationContext.signal?.aborted) {
      return resultForStructured({
        ok: false,
        error: {
          code: CUT_MCP_ERROR_CODES.requestAborted,
          message: `Request for ${name} was aborted`,
        },
      });
    }

    try {
      const result = await entry.handler!(actualInput, invocationContext);
      // Force an early serialization check so a handler cannot accidentally
      // return a value that the MCP JSON transport cannot encode.
      try {
        jsonText(result);
      } catch (error) {
        throw new CutMcpError(
          CUT_MCP_ERROR_CODES.serializationFailed,
          `Result from ${name} is not JSON serializable`,
          error instanceof Error ? error.message : String(error),
        );
      }
      return resultForStructured({ ok: true, result });
    } catch (error) {
      return resultForStructured({ ok: false, error: errorShape(asError(error)) });
    }
  }
}

export function createCutMcpRouter(
  handlers: CutMcpHandlers = {},
  options: CutMcpRouterOptions = {},
): CutMcpRouter {
  return new CutMcpRouter(handlers, options);
}

/**
 * Register the router on the SDK's low-level Server.  The low-level API is
 * intentional: SDK 1.x's convenience API accepts Zod schemas, while cut's
 * public contract is JSON Schema and must remain host-implementation-neutral.
 */
export function registerCutMcpTools(
  server: Server,
  router: CutMcpRouter,
): void {
  server.setRequestHandler(ListToolsRequestSchema, () => ({
    tools: router.listTools(),
  }));
  server.setRequestHandler(CallToolRequestSchema, async (request, extra) => {
    const result = await router.call(
      request.params.name,
      request.params.arguments ?? {},
      {
        signal: extra.signal,
        requestId: extra.requestId,
        ...(extra.sessionId !== undefined ? { sessionId: extra.sessionId } : {}),
      },
    );
    return result as unknown as CallToolResult;
  });
}

export interface CutMcpServer {
  /** SDK protocol server; connect it to StdioServerTransport when desired. */
  readonly server: Server;
  readonly protocolServer: Server;
  readonly router: CutMcpRouter;
  readonly listTools: () => CutMcpToolDefinition[];
  readonly call: (
    name: string,
    input?: unknown,
  ) => Promise<CutMcpCallResult>;
  readonly callTool: (
    name: string,
    input?: unknown,
  ) => Promise<CutMcpCallResult>;
  connect(transport: Transport): Promise<void>;
  close(): Promise<void>;
}

/** Create a transport-neutral SDK server backed by injected cut capabilities. */
export function createCutMcpServer(
  handlers: CutMcpHandlers = {},
  options: CutMcpServerOptions = {},
): CutMcpServer {
  const router = new CutMcpRouter(handlers, options);
  const protocolServer = new Server(
    {
      name: options.name ?? "cut",
      version: options.version ?? "0.1.0",
    },
    {
      capabilities: { tools: { listChanged: false } },
      ...(options.instructions !== undefined ? { instructions: options.instructions } : {}),
    },
  );
  registerCutMcpTools(protocolServer, router);
  return {
    server: protocolServer,
    protocolServer,
    router,
    listTools: () => router.listTools(),
    call: (name, input = {}) => router.call(name, input),
    callTool: (name, input = {}) => router.call(name, input),
    connect: (transport) => protocolServer.connect(transport),
    close: () => protocolServer.close(),
  };
}

export const createMcpServer = createCutMcpServer;
