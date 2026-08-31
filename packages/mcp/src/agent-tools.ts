import type { ToolImpl } from "./tool-impl.js";
import type { CutMcpProjectHost } from "./host.js";
import {
  CUT_MCP_TOOL_SCHEMAS,
  createCutMcpRouter,
  type CutMcpCallResult,
} from "./server.js";
import { createSkillReadTool, type Skill } from "@cut/skills";

export interface CutAgentToolsOptions {
  /** Loaded playbooks exposed through skill_read (never via bash or paths). */
  skills?: readonly Skill[];
}

function mcpError(result: CutMcpCallResult): Error {
  const structured = result.structuredContent as {
    error?: { code?: unknown; message?: unknown; details?: unknown };
  };
  const error = new Error(
    structured.error?.message !== undefined
      ? String(structured.error.message)
      : result.content[0]?.text ?? "MCP tool call failed",
  );
  const coded = error as Error & { code?: unknown; details?: unknown };
  if (structured.error?.code !== undefined) coded.code = structured.error.code;
  if (structured.error?.details !== undefined) coded.details = structured.error.details;
  return error;
}

async function callMcp(
  router: ReturnType<typeof createCutMcpRouter>,
  capability: string,
  input: unknown = {},
): Promise<unknown> {
  const result = await router.call(capability, input);
  if (result.isError) throw mcpError(result);
  return (result.structuredContent as { result: unknown }).result;
}

function hasCapability(host: CutMcpProjectHost, capability: string): boolean {
  const router = createCutMcpRouter(host.handlers);
  return router.hasTool(capability);
}

/** Build the agent-facing tool map backed by one project-scoped MCP host. */
export function createCutAgentTools(
  host: CutMcpProjectHost,
  options?: CutAgentToolsOptions,
): Record<string, ToolImpl> {
  const router = createCutMcpRouter(host.handlers);

  const tools: Record<string, ToolImpl> = {
    fetch: {
      description: "Download media from a URL and append it to the project timeline.",
      inputJsonSchema: CUT_MCP_TOOL_SCHEMAS["media.fetch"],
      execute: (input: unknown) => callMcp(router, "media.fetch", input),
    },
    probe: {
      description: CUT_MCP_TOOL_SCHEMAS["media.probe"].description as string,
      inputJsonSchema: CUT_MCP_TOOL_SCHEMAS["media.probe"],
      async execute(input: unknown) {
        const parsed = input as { action?: string };
        const result = (await callMcp(router, "media.probe", input)) as {
          action?: string;
          text?: string;
          frames?: Array<{ time: number; path: string; imageBase64: string; mediaType: string }>;
        };
        if (parsed.action === "frames" && result.action === "frames" && Array.isArray(result.frames)) {
          return {
            type: "multimodal_tool_result",
            text: result.text ?? "",
            metadata: result.frames.map(({ time, path: framePath }) => ({ time, path: framePath })),
            images: result.frames.map(({ imageBase64, mediaType }) => ({
              data: imageBase64,
              mediaType,
            })),
          };
        }
        return result;
      },
    },
    timeline: {
      description: "Read the current v2 project timeline before editing.",
      inputJsonSchema: CUT_MCP_TOOL_SCHEMAS["timeline.query"],
      execute: (input: unknown = {}) => callMcp(router, "timeline.query", input),
    },
    project_edit: {
      description:
        "Apply one canonical v2 project command or an atomic command batch with expectedRevision conflict protection.",
      inputJsonSchema: CUT_MCP_TOOL_SCHEMAS["timeline.apply"],
      execute: (input: unknown) => callMcp(router, "timeline.apply", input ?? {}),
    },
    render: {
      description: "Render the current project; set draft=true for a quick preview.",
      inputJsonSchema: CUT_MCP_TOOL_SCHEMAS["render.start"],
      async execute(input: { draft?: boolean } = {}) {
        const started = (await callMcp(
          router,
          input.draft ? "render.preview" : "render.start",
          { draft: input.draft ?? false },
        )) as { jobId: string; output?: string; status?: unknown };
        const finished = (await callMcp(router, "render.result", { jobId: started.jobId })) as {
          jobId: string;
          output?: string;
          status?: unknown;
        };
        return {
          output: finished.output ?? started.output,
          jobId: started.jobId,
          status: finished.status ?? started.status,
        };
      },
    },
    timeline_undo: {
      description: "Undo one or more project commands on the current host.",
      inputJsonSchema: CUT_MCP_TOOL_SCHEMAS["timeline.undo"],
      execute: (input: unknown = {}) => callMcp(router, "timeline.undo", input),
    },
    timeline_redo: {
      description: "Redo one or more project commands on the current host.",
      inputJsonSchema: CUT_MCP_TOOL_SCHEMAS["timeline.redo"],
      execute: (input: unknown = {}) => callMcp(router, "timeline.redo", input),
    },
    skill_read: createSkillReadTool(options?.skills ?? []),
  };

  if (hasCapability(host, "vision.track_objects")) {
    tools.track_objects = {
      description: CUT_MCP_TOOL_SCHEMAS["vision.track_objects"].description as string,
      inputJsonSchema: CUT_MCP_TOOL_SCHEMAS["vision.track_objects"],
      execute: (input: unknown) => callMcp(router, "vision.track_objects", input),
    };
  }

  return tools;
}
