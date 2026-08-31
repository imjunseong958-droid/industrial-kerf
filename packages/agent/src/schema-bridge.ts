import type Anthropic from "@anthropic-ai/sdk";
import { z } from "zod/v4";
import {
  EDIT_INPUT_JSON_SCHEMA,
  PROJECT_EDIT_INPUT_JSON_SCHEMA,
  TRACK_OBJECTS_INPUT_JSON_SCHEMA,
  TIMELINE_INPUT_JSON_SCHEMA,
  TOOL_NAMES,
  type CutTools,
  type ToolName,
} from "./tools.js";

/**
 * A concrete tool implementation handed to AgentSession. Only the tools
 * present in the map are offered to the model.
 *
 * The map holds the built-in CutTools contracts plus any number of
 * extension tools (e.g. MCP tools under `mcp_<server>_<tool>` names), which
 * must carry their own description and inputJsonSchema.
 */
export type ToolMap = Partial<Record<ToolName, ToolImpl<any, any>>> & {
  [extension: string]: ToolImpl<any, any> | undefined;
};

/** MIME types accepted by the multimodal tool-result contract. */
export type ToolResultImageMediaType = "image/png" | "image/jpeg";

/**
 * One local image returned as part of a tool result.
 *
 * `data` is either the raw base64-encoded image bytes (in which case
 * `mediaType` is required) or a `data:image/png;base64,...` /
 * `data:image/jpeg;base64,...` URL (in which case the media type is inferred).
 * Local file paths and remote URLs are not accepted here: the
 * session owns the bytes that it sends to a provider and can therefore bound
 * and validate them before they enter the conversation history.
 */
export interface ToolResultImage {
  data: string;
  mediaType?: ToolResultImageMediaType;
}

/**
 * Special return shape for tools that need to show frames to the model.
 *
 * A ToolImpl may return this exact discriminated shape instead of its normal
 * JSON result. `text` is optional human-readable metadata, while `metadata`
 * is optional JSON-serializable metadata. At least one image is required.
 * AgentSession converts it to an Anthropic tool_result containing a text block
 * and native base64 image blocks; provider adapters translate those blocks for
 * OpenAI-compatible APIs. Ordinary tool return values are still JSON strings.
 */
export interface MultimodalToolResult {
  type: "multimodal_tool_result";
  text?: string;
  metadata?: unknown;
  /** Must contain at least one image at runtime. */
  images: readonly ToolResultImage[];
}

/**
 * A concrete tool implementation handed to AgentSession. Only the tools
 * present in the map are offered to the model.
 */
export interface ToolImpl<I = unknown, O = unknown> {
  /** Optional override of the tool description shown to the model. */
  description?: string;
  /** Optional zod schema; converted to JSON Schema for the API. */
  inputSchema?: z.ZodType<I>;
  /** Optional explicit JSON Schema (wins over inputSchema). */
  inputJsonSchema?: Record<string, unknown>;
  execute(input: I): Promise<O>;
}

/** Default descriptions + JSON schemas derived from the CutTools contracts. */
const DEFAULT_TOOL_DEFS: Record<
  ToolName,
  { description: string; input_schema: Record<string, unknown> }
> = {
  fetch: {
    description: "Fetch a video/media resource by URL and store it locally.",
    input_schema: {
      type: "object",
      properties: { url: { type: "string", description: "Source URL" } },
      required: ["url"],
      additionalProperties: false,
    },
  },
  probe: {
    description:
      'Inspect a local media file: "info", "silence" ranges, "scenes", or extracted "frames". ' +
      'The frames action returns up to 8 visually inspectable PNG images plus exact {time,path} metadata; ' +
      'use it to infer normalized 0..1 crop coordinates from a screenshot before calling project_edit.',
    input_schema: {
      type: "object",
      properties: {
        action: {
          type: "string",
          enum: ["info", "silence", "scenes", "frames"],
        },
        src: { type: "string", description: "Local path to media file" },
        opts: {
          type: "object",
          properties: {
            frames: {
              type: "integer",
              minimum: 1,
              maximum: 8,
              description: "Number of PNG frames to return visually (maximum 8).",
            },
            noiseDb: { type: "number" },
            minDuration: { type: "number" },
          },
          additionalProperties: false,
        },
      },
      required: ["action", "src"],
      additionalProperties: false,
    },
  },
  edit: {
    description:
      "Compatibility alias for legacy v1 timeline edits. Hosts may omit this tool; prefer project_edit. " +
      "When present, accepts the same op/ops shape as project_edit legacy mode.",
    input_schema: EDIT_INPUT_JSON_SCHEMA,
  },
  render: {
    description:
      "Render the timeline. draft=true for a fast low-quality preview render.",
    input_schema: {
      type: "object",
      properties: { draft: { type: "boolean" } },
      additionalProperties: false,
    },
  },
  bash: {
    description:
      "Run a shell command with cwd set to the project workspace. This is not a filesystem sandbox: commands retain host process permissions, so use it deliberately.",
    input_schema: {
      type: "object",
      properties: { command: { type: "string" } },
      required: ["command"],
      additionalProperties: false,
    },
  },
  timeline: {
    description:
      "Read the validated current project timeline, source ids, clip ids, output positions, and durations. " +
      "Use this before project_edit so operations target real ids; it is read-only.",
    input_schema: TIMELINE_INPUT_JSON_SCHEMA,
  },
  track_objects: {
    description:
      "Track configured cam (streamer camera), chat (chat overlay), and content (main gameplay/stream canvas) objects and return class-separated ROI/crop planning data. " +
      "Use plan.keyframesByClass.content when framing the main picture. Weights and labels come only from project/user configuration; they are not selectable in tool input.",
    input_schema: TRACK_OBJECTS_INPUT_JSON_SCHEMA,
  },
  project_edit: {
    description:
      "Apply canonical v2 command/commands atomically with expectedRevision conflict protection, " +
      "or legacy op/ops which the host translates to project changes.",
    input_schema: PROJECT_EDIT_INPUT_JSON_SCHEMA,
  },
  skill_read: {
    description: "Read a loaded playbook body by name.",
    input_schema: {
      type: "object",
      properties: { name: { type: "string", minLength: 1 } },
      required: ["name"],
      additionalProperties: false,
    },
  },
  timeline_undo: {
    description: "Undo one or more project commands on the current host.",
    input_schema: {
      type: "object",
      properties: {
        steps: { type: "integer", minimum: 1, maximum: 100 },
        expectedRevision: { type: "integer", minimum: 0 },
      },
      additionalProperties: false,
    },
  },
  timeline_redo: {
    description: "Redo one or more previously undone project commands.",
    input_schema: {
      type: "object",
      properties: {
        steps: { type: "integer", minimum: 1, maximum: 100 },
        expectedRevision: { type: "integer", minimum: 0 },
      },
      additionalProperties: false,
    },
  },
};

/** Convert one ToolMap entry into an Anthropic tool definition. */
export function toAnthropicTool(
  name: string,
  impl?: ToolImpl,
): Anthropic.Tool {
  const def = (DEFAULT_TOOL_DEFS as Record<string, { description: string; input_schema: Record<string, unknown> }>)[name];
  let input_schema: Record<string, unknown>;
  if (impl?.inputJsonSchema) {
    input_schema = impl.inputJsonSchema;
  } else if (impl?.inputSchema instanceof z.ZodType) {
    input_schema = z.toJSONSchema(impl.inputSchema) as Record<string, unknown>;
  } else {
    input_schema = def?.input_schema ?? { type: "object", properties: {} };
  }
  return {
    name,
    description: impl?.description ?? def?.description ?? `Call the ${name} tool.`,
    input_schema: input_schema as unknown as Anthropic.Tool.InputSchema,
  };
}

/** Build the full Anthropic tools array for the given tool map. */
export function buildToolsParam(toolMap: ToolMap): Anthropic.Tool[] {
  const tools: Anthropic.Tool[] = [];
  for (const [name, impl] of Object.entries(toolMap)) {
    if (impl == null) continue;
    tools.push(toAnthropicTool(name, impl as ToolImpl));
  }
  return tools;
}

/**
 * Registry-only helper: schemas for safe built-in contracts by default.
 * Optional implementations (including bash) are included only when a
 * concrete registry actually exposes them.
 */
export function cutToolsSchemas(
  registry?: CutTools,
): Array<{ name: ToolName } & Pick<Anthropic.Tool, "description" | "input_schema">> {
  const names = registry
    ? Object.keys(registry).filter((name): name is ToolName => name in DEFAULT_TOOL_DEFS)
    : [...TOOL_NAMES];
  return names.map((name) => ({
    name,
    description: DEFAULT_TOOL_DEFS[name]!.description,
    input_schema: DEFAULT_TOOL_DEFS[name]!.input_schema as Anthropic.Tool.InputSchema,
  }));
}
