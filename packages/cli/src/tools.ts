/**
 * CutTools implementations wiring @cut/mcp project host into the ToolMap
 * shape AgentSession consumes. Bash remains CLI-only.
 */
import { execFile } from "node:child_process";
import path from "node:path";
import { z } from "zod";
import { fetchVideo } from "@cut/fetcher";
import { info, silenceDetect, detectScenes, extractFrames, type RenderJobManager } from "@cut/media";
import {
  createCutAgentTools,
  createCutMcpProjectHost,
  type CutMcpRenderHandlers,
} from "@cut/mcp";
import type { ToolImpl, ToolName } from "@cut/agent";
import type { Skill } from "@cut/skills";
import type { Yolo26Tracker, Yolo26TrackingConfig } from "@cut/vision";

export interface CutToolHooks {
  onProgress?: (tool: ToolName, ratio: number) => void;
  onRenderDone?: (output: string) => void;
}

export interface BuildToolsOptions {
  dir: string;
  hooks?: CutToolHooks;
  /** Optional yt-dlp cookie file path passed through to fetchVideo. */
  cookies?: string;
  /** Explicit, trusted vision setup. Model weights never come from tool input. */
  vision?: Yolo26TrackingConfig | Yolo26Tracker | VisionBuildOptions;
  /** Aliases when vision config is stored separately. */
  visionConfig?: Yolo26TrackingConfig;
  visionTracker?: Yolo26Tracker;
  /** Shell execution is disabled unless this is true or CUT_ENABLE_BASH=1. */
  enableBash?: boolean;
  /** Injectable render handlers for tests/embedding. */
  render?: CutMcpRenderHandlers;
  /** Injectable scheduler used by the built-in render handlers. */
  renderManager?: RenderJobManager;
  /** Loaded playbooks passed through to skill_read. */
  skills?: Skill[];
}

export interface VisionBuildOptions {
  readonly config?: Yolo26TrackingConfig;
  readonly tracker?: Yolo26Tracker;
}

const BashInputSchema = z.object({ command: z.string() }).strict();

const BASH_INPUT_JSON_SCHEMA: Record<string, unknown> = {
  type: "object",
  description:
    "Run a shell command with cwd set to the session directory. This is not a filesystem sandbox; commands retain host process permissions.",
  properties: { command: { type: "string", description: "Shell command run in the session directory." } },
  required: ["command"],
  additionalProperties: false,
};

function parseBashInput(input: unknown): { command: string } {
  const result = BashInputSchema.safeParse(input);
  if (!result.success) {
    throw new Error(`invalid bash input: ${result.error.message}`);
  }
  return result.data;
}

/** Build a ToolMap of concrete tool implementations rooted at `dir`. */
export function buildCutTools(opts: BuildToolsOptions) {
  const { hooks = {} } = opts;
  const host = createCutMcpProjectHost({
    dir: opts.dir,
    fetchVideo,
    info,
    extractFrames,
    silenceDetect,
    detectScenes,
    ...(opts.cookies ? { cookies: opts.cookies } : {}),
    vision: opts.visionTracker ?? opts.visionConfig ?? opts.vision,
    ...(opts.renderManager ? { renderManager: opts.renderManager } : {}),
    ...(opts.render ? { render: opts.render } : {}),
  });
  const tools = createCutAgentTools(host, { skills: opts.skills });
  const bashEnabled = opts.enableBash === true || process.env["CUT_ENABLE_BASH"] === "1";

  if (hooks.onRenderDone && tools.render) {
    const baseRender = tools.render;
    tools.render = {
      ...baseRender,
      async execute(input: unknown) {
        const result = await baseRender.execute(input);
        const output = (result as { output?: unknown } | undefined)?.output;
        if (typeof output === "string" && output.length > 0) {
          hooks.onRenderDone!(output);
        }
        return result;
      },
    };
  }

  const bash = {
    inputJsonSchema: BASH_INPUT_JSON_SCHEMA,
    async execute(input: unknown) {
      const parsed = parseBashInput(input);
      return new Promise<{ stdout: string; stderr: string; exitCode: number }>((resolve) => {
        execFile(
          "/bin/sh",
          ["-c", parsed.command],
          { cwd: path.resolve(opts.dir), timeout: 120_000, maxBuffer: 8 * 1024 * 1024 },
          (err, stdout, stderr) => {
            const code = err == null ? 0 : ((err as NodeJS.ErrnoException & { code?: number }).code ?? 1);
            resolve({
              stdout: String(stdout).slice(0, 20_000),
              stderr: String(stderr).slice(0, 20_000),
              exitCode: typeof code === "number" ? code : 1,
            });
          },
        );
      });
    },
  } satisfies ToolImpl<{ command: string }, { stdout: string; stderr: string; exitCode: number }>;

  return { ...tools, ...(bashEnabled ? { bash } : {}) };
}
