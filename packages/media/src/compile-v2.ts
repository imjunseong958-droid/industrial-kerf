/**
 * Version 2 filtergraph compiler. TypeScript wrapper that calls the native cut-engine binary.
 */
import type { Project } from "@cut/timeline";
import { CutEngineError, invokeCutEngineJson } from "@cut/engine";

/** A compiled v2 graph has the same argv shape as the v1 compiler. */
export interface CompiledV2FilterGraph {
  /** Full ffmpeg argv array (spawn-safe, no shell). */
  argv: string[];
  /** Human-readable rendering of the same command. */
  command: string;
  /** The filter_complex string itself. */
  filterComplex: string;
  /** Effective output duration in seconds. */
  duration: number;
  /** Effective output canvas width. */
  width: number;
  /** Effective output canvas height. */
  height: number;
}

export interface CompileV2Options {
  /** Optional filters applied to the finished composited video. */
  videoPostFilters?: string[];
  /** Include embedded audio from video tracks even when explicit audio tracks exist. */
  includeEmbeddedAudio?: boolean;
}

/** Raised when a v2 feature cannot be represented by this ffmpeg subset. */
export class UnsupportedV2FeatureError extends Error {
  readonly feature: string;

  constructor(feature: string, detail: string) {
    super(`Unsupported v2 ${feature}: ${detail}`);
    this.name = "UnsupportedV2FeatureError";
    this.feature = feature;
  }
}

const UNSUPPORTED_V2_RE = /^Unsupported v2 ([^:]+): (.*)$/;

function compileV2Args(opts: CompileV2Options = {}): string[] {
  const args = ["compile-v2"];
  if (opts.includeEmbeddedAudio !== undefined) {
    args.push("--include-embedded-audio", String(opts.includeEmbeddedAudio));
  }
  for (const filter of opts.videoPostFilters ?? []) {
    args.push("--post-filter", filter);
  }
  return args;
}

function mapCompileV2Error(error: unknown): never {
  if (error instanceof CutEngineError) {
    const message = error.message.replace(/^Error: /, "");
    const match = UNSUPPORTED_V2_RE.exec(message);
    if (match) throw new UnsupportedV2FeatureError(match[1]!, match[2]!);
    throw new Error(message);
  }
  throw error;
}

function invokeCompileV2(
  project: Project,
  opts: CompileV2Options = {},
): CompiledV2FilterGraph {
  try {
    return invokeCutEngineJson<CompiledV2FilterGraph>(compileV2Args(opts), {
      input: JSON.stringify(project),
    });
  } catch (error) {
    mapCompileV2Error(error);
  }
}

/**
 * Compile a version 2 project into a timeline-positioned compositing graph.
 */
export function compileFilterGraphV2(
  project: Project,
  opts: CompileV2Options = {},
): CompiledV2FilterGraph {
  return invokeCompileV2(project, opts);
}

export const compileV2 = compileFilterGraphV2;
export const compileV2FilterGraph = compileFilterGraphV2;
export const compileProject = compileFilterGraphV2;
export const compileTimelineV2 = compileFilterGraphV2;

/** Pure duration helper for v2 projects. */
export function timelineDurationV2(project: Project): number {
  try {
    return invokeCutEngineJson<{ duration: number }>(["duration-v2"], {
      input: JSON.stringify(project),
    }).duration;
  } catch (error) {
    mapCompileV2Error(error);
  }
}
