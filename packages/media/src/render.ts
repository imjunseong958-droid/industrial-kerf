/**
 * render() — spawns ffmpeg with a compiled filtergraph, parses progress
 * from stderr `time=HH:MM:SS.xx` lines and reports via onProgress.
 */
import { spawn } from "node:child_process";
import type { Timeline } from "@cut/timeline";
import { compileFilterGraph, timelineDuration } from "./compile.js";

export interface RenderOptions {
  output: string;
  /** Fast preview mode: h=360, fps=15, crf=35, ultrafast. */
  draft?: boolean;
  /** Progress callback, ratio in [0,1]. */
  onProgress?: (ratio: number) => void;
}

/** Parse ffmpeg's `time=HH:MM:SS.xx` (or `time=N.NN`) into seconds. */
export function parseTimeSeconds(timeStr: string): number | undefined {
  const hmss = timeStr.match(/^(\d+):(\d+):(\d+(?:\.\d+)?)$/);
  if (hmss) {
    return (
      Number(hmss[1]) * 3600 + Number(hmss[2]) * 60 + Number(hmss[3])
    );
  }
  const n = Number(timeStr);
  return Number.isFinite(n) ? n : undefined;
}

export function renderArgs(
  timeline: Timeline,
  opts: RenderOptions,
): string[] {
  const compiled = compileFilterGraph(
    timeline,
    opts.draft ? { videoPostFilters: ["scale=-2:360", "fps=15"] } : {},
  );
  // compiled.argv[0] is "ffmpeg"; spawn supplies it. Drop it plus the
  // trailing default -r <fps> pair (re-added below with codec args).
  const base = compiled.argv.slice(1, -2);
  const codec = opts.draft
    ? ["-crf", "35", "-preset", "ultrafast"]
    : ["-crf", "20", "-preset", "medium"];
  return [
    ...base,
    ...codec,
    "-r",
    String(timeline.fps),
    opts.output,
  ];
}

export async function render(
  timeline: Timeline,
  opts: RenderOptions,
): Promise<void> {
  const total = timelineDuration(timeline);
  const args = renderArgs(timeline, opts);

  return new Promise((resolve, reject) => {
    const child = spawn("ffmpeg", args, { stdio: ["ignore", "ignore", "pipe"] });
    let stderr = "";
    child.stderr.on("data", (d: Buffer) => {
      const text = d.toString();
      stderr += text;
      if (stderr.length > 100_000) stderr = stderr.slice(-50_000);
      if (opts.onProgress && total > 0) {
        for (const m of text.matchAll(/time=(\S+)/g)) {
          const t = parseTimeSeconds(m[1]!);
          if (t !== undefined) {
            opts.onProgress(Math.min(1, Math.max(0, t / total)));
          }
        }
      }
    });
    child.on("error", reject);
    child.on("close", (code) => {
      if (code === 0) resolve();
      else
        reject(
          new Error(
            `ffmpeg exited with code ${code}\nargs: ${args.join(" ")}\nstderr: ${stderr.slice(-2000)}`,
          ),
        );
    });
  });
}
