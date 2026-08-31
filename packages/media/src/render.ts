/**
 * render() spawns cut-engine render-v1 --run. Progress arrives as stderr JSON lines.
 */
import type { Timeline } from "@cut/timeline";
import { invokeCutEngineJson, spawnCutEngineJson } from "@cut/engine";

export interface RenderOptions {
  output: string;
  /** Fast preview mode: h=360, fps=15, crf=35, ultrafast. */
  draft?: boolean;
  /** Progress callback, ratio in [0,1]. */
  onProgress?: (ratio: number) => void;
  /** Abort an in-flight render. */
  signal?: AbortSignal;
}

function renderV1Args(timeline: Timeline, opts: RenderOptions): string[] {
  const args = ["render-v1", "--output", opts.output];
  if (opts.draft) args.push("--draft");
  return invokeCutEngineJson<{ args: string[] }>(args, {
    input: JSON.stringify(timeline),
  }).args;
}

export function renderArgs(
  timeline: Timeline,
  opts: RenderOptions,
): string[] {
  return renderV1Args(timeline, opts);
}

export async function render(
  timeline: Timeline,
  opts: RenderOptions,
): Promise<void> {
  const args = ["render-v1", "--run", "--output", opts.output];
  if (opts.draft) args.push("--draft");
  await spawnCutEngineJson<{ ok: boolean }>(args, {
    input: JSON.stringify(timeline),
    signal: opts.signal,
    timeoutMs: 3_600_000,
    onProgress: opts.onProgress,
  });
}
