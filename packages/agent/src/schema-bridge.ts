import type Anthropic from "@anthropic-ai/sdk";
import { z } from "zod/v4";
import type { CutTools, ToolName } from "./tools.js";

/**
 * A concrete tool implementation handed to AgentSession. Only the tools
 * present in the map are offered to the model.
 *
 * The map holds the five built-in CutTools contracts plus any number of
 * extension tools (e.g. MCP tools under `mcp_<server>_<tool>` names), which
 * must carry their own description and inputJsonSchema.
 */
export type ToolMap = Partial<Record<ToolName, ToolImpl<any, any>>> & {
  [extension: string]: ToolImpl<any, any> | undefined;
};

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
      'Inspect a local media file: "info", "silence" ranges, "scenes", or extracted "frames".',
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
            frames: { type: "number" },
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
      "Apply an edit operation to the timeline and return the updated Timeline JSON. " +
      "Ops: trim, split, remove_ranges, set_speed, set_volume, " +
      "add_subtitles (burn in a subtitle file: { op: 'add_subtitles', path, style?: { fontSize, fontColor, outlineColor, position: 'bottom'|'center'|'top', marginV } }), " +
      "remove_overlay ({ op: 'remove_overlay', id }).",
    input_schema: {
      type: "object",
      properties: { op: { type: "object", description: "EditOp union" } },
      required: ["op"],
      additionalProperties: false,
    },
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
    description: "Run a shell command in the project workspace.",
    input_schema: {
      type: "object",
      properties: { command: { type: "string" } },
      required: ["command"],
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
    description: impl?.description ?? def!.description,
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

/** Registry-only helper: schemas for all five CutTools contracts. */
export function cutToolsSchemas(
  _registry?: CutTools,
): Array<{ name: ToolName } & Pick<Anthropic.Tool, "description" | "input_schema">> {
  return (Object.keys(DEFAULT_TOOL_DEFS) as ToolName[]).map((name) => ({
    name,
    description: DEFAULT_TOOL_DEFS[name]!.description,
    input_schema: DEFAULT_TOOL_DEFS[name]!.input_schema as Anthropic.Tool.InputSchema,
  }));
}
