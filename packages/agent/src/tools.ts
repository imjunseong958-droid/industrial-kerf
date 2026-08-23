import type { EditOp, Timeline } from "@cut/timeline";

// ---------------------------------------------------------------------------
// Tool contracts for the @cut agent
// ---------------------------------------------------------------------------

/** A tool the agent can call. `input` is JSON-schema-ish (typed structurally). */
export interface Tool<I = unknown, O = unknown> {
  name: string;
  description: string;
  input: I;
  // eslint-disable-next-line @typescript-eslint/no-unused-vars
  output: O;
}

// --- fetch -----------------------------------------------------------------

export interface FetchInput {
  url: string;
}
export interface FetchOutput {
  path: string;
  subsPath?: string;
  title?: string;
}

// --- probe -----------------------------------------------------------------

export interface ProbeInput {
  action: "info" | "silence" | "scenes" | "frames";
  src: string;
  opts?: {
    /** for "frames": number of frames to extract (default 10) */
    frames?: number;
    /** for "silence": noise threshold in dB (default -30) */
    noiseDb?: number;
    /** for "silence": minimum silence duration in seconds (default 0.5) */
    minDuration?: number;
  };
}
export interface ProbeOutput {
  info: Record<string, unknown>;
  ranges?: Array<{ start: number; end: number }>;
  frames?: string[];
}

// --- edit ------------------------------------------------------------------

export interface EditInput {
  op: EditOp;
}
export type EditOutput = Timeline;

// --- render ----------------------------------------------------------------

export interface RenderInput {
  /** draft mode: fast, lower-quality proxy render */
  draft?: boolean;
}
export interface RenderOutput {
  output: string;
}

// --- bash ------------------------------------------------------------------

export interface BashInput {
  command: string;
}
export interface BashOutput {
  stdout: string;
  stderr: string;
  exitCode: number;
}

// --- registry --------------------------------------------------------------

export type CutTools = {
  fetch: Tool<FetchInput, FetchOutput>;
  probe: Tool<ProbeInput, ProbeOutput>;
  edit: Tool<EditInput, EditOutput>;
  render: Tool<RenderInput, RenderOutput>;
  bash: Tool<BashInput, BashOutput>;
};

export const TOOL_NAMES = ["fetch", "probe", "edit", "render", "bash"] as const;
export type ToolName = (typeof TOOL_NAMES)[number];
