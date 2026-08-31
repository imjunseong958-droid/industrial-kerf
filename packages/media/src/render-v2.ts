/** Runtime cut-engine wrapper for the version 2 project IR. */
import type { Project } from "@cut/timeline";
import { invokeCutEngineJson, spawnCutEngineJson } from "@cut/engine";
import type { CompiledV2FilterGraph } from "./compile-v2.js";

export interface RenderV2Options {
  output: string;
  /** Fast preview mode: h=360, 15fps, crf35, ultrafast. */
  draft?: boolean;
  /** Opt in to embedded video-track audio alongside explicit audio tracks. */
  includeEmbeddedAudio?: boolean;
  /**
   * Encode policy. `undefined` lets the engine pick hardware when available.
   * `true` forces a hardware attempt; `false` forces libx264.
   */
  preferHardware?: boolean;
  /** Progress callback, ratio in [0,1]. */
  onProgress?: (ratio: number) => void;
  /** Abort an in-flight render. */
  signal?: AbortSignal;
}

function hardwareEncodeArgs(preferHardware: boolean | undefined): string[] {
  if (preferHardware === true) return ["--prefer-hardware"];
  if (preferHardware === false) return ["--software"];
  return [];
}

/**
 * Build spawn-safe arguments for a v2 project via the native engine
 * (hwaccel decode prefix, auto hardware encode, filtergraph).
 */
export function renderArgsV2(
  project: Project,
  opts: RenderV2Options,
): string[] {
  const args = ["render-v2", "--output", opts.output, ...hardwareEncodeArgs(opts.preferHardware)];
  if (opts.draft) args.push("--draft");
  if (opts.includeEmbeddedAudio !== undefined) {
    args.push("--include-embedded-audio", String(opts.includeEmbeddedAudio));
  }
  return invokeCutEngineJson<{ args: string[] }>(args, {
    input: JSON.stringify(project),
  }).args;
}

/** Render a v2 project with progress parsed from cut-engine stderr JSON lines. */
export async function renderV2(
  project: Project,
  opts: RenderV2Options,
): Promise<void> {
  const args = ["render-v2", "--run", "--output", opts.output, ...hardwareEncodeArgs(opts.preferHardware)];
  if (opts.draft) args.push("--draft");
  if (opts.includeEmbeddedAudio !== undefined) {
    args.push("--include-embedded-audio", String(opts.includeEmbeddedAudio));
  }
  await spawnCutEngineJson<{ ok: boolean }>(args, {
    input: JSON.stringify(project),
    signal: opts.signal,
    timeoutMs: 3_600_000,
    onProgress: opts.onProgress,
  });
}

export const renderProjectV2 = renderV2;
export const renderTimelineV2 = renderV2;
export const renderV2Args = renderArgsV2;

/** Re-export the compiler result type. */
export type { CompiledV2FilterGraph };
