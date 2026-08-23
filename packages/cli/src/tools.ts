/**
 * CutTools implementations wiring @cut/media, @cut/timeline, @cut/fetcher
 * into the ToolMap shape AgentSession consumes.
 */
import { execFile } from "node:child_process";
import path from "node:path";
import {
  EditOpSchema,
  applyEdit,
  type Timeline,
} from "@cut/timeline";
import {
  detectScenes,
  extractFrames,
  info,
  silenceDetect,
  timelineDuration,
  render,
} from "@cut/media";
import { fetchVideo } from "@cut/fetcher";
import type { ToolImpl, ToolName } from "@cut/agent";
import {
  initialTimeline,
  loadTimeline,
  saveTimeline,
  framesDir,
} from "./state.js";
import { renderTimelineGantt, progressBar } from "./gantt.js";
import { showFrame } from "./preview.js";

export interface CutToolHooks {
  onProgress?: (tool: ToolName, ratio: number) => void;
  onRenderDone?: (output: string) => void;
}

export interface BuildToolsOptions {
  dir: string;
  hooks?: CutToolHooks;
  /** Optional yt-dlp cookie file path passed through to fetchVideo. */
  cookies?: string;
}

function resolveIn(dir: string, src: string): string {
  return path.isAbsolute(src) ? src : path.join(path.resolve(dir), src);
}

/** Build a ToolMap of concrete tool implementations rooted at `dir`. */
export function buildCutTools(opts: BuildToolsOptions) {
  const { dir, hooks = {}, cookies } = opts;

  const fetch = {
    async execute(input: { url: string }) {
      const result = await fetchVideo(input.url, path.resolve(dir), {
        ...(cookies ? { cookies } : {}),
        onProgress: (pct) =>
          process.stderr.write(`\rfetch ${progressBar(pct / 100)}\r`),
      });
      // Auto-create an initial timeline for the fetched media.
      const meta = await info(result.path);
      const existing = await loadTimeline(dir);
      if (!existing || existing.sources.every((s) => s.path !== result.path)) {
        const tl = initialTimeline(
          `src${(existing?.sources.length ?? 0) + 1}`,
          result.path,
          meta,
          result.subsPath,
        );
        await saveTimeline(dir, tl);
        console.log("\n" + renderTimelineGantt(tl));
      }
      return result;
    },
  } satisfies ToolImpl<{ url: string }, { path: string; subsPath?: string; title?: string }>;

  const probe = {
    async execute(input: {
      action: "info" | "silence" | "scenes" | "frames";
      src: string;
      opts?: { frames?: number; noiseDb?: number; minDuration?: number };
    }) {
      const src = resolveIn(dir, input.src);
      if (input.action === "info") {
        return { info: await info(src) as unknown as Record<string, unknown> };
      }
      if (input.action === "silence") {
        const ranges = await silenceDetect(src, {
          ...(input.opts?.noiseDb !== undefined ? { thresholdDb: input.opts.noiseDb } : {}),
          ...(input.opts?.minDuration !== undefined ? { minDurationSec: input.opts.minDuration } : {}),
        });
        return {
          info: {} as Record<string, unknown>,
          ranges: ranges.map((r) => ({
            start: r.start,
            end: r.end ?? Number.POSITIVE_INFINITY,
          })),
        };
      }
      if (input.action === "scenes") {
        const ranges = await detectScenes(src);
        return {
          info: {} as Record<string, unknown>,
          ranges: ranges.map((r) => ({
            start: r.start,
            end: r.end ?? Number.POSITIVE_INFINITY,
          })),
        };
      }
      const n = input.opts?.frames ?? 10;
      const meta = await info(src);
      const duration = meta.duration ?? 60;
      const times = Array.from({ length: n }, (_, i) => (duration * i) / n + duration / (2 * n));
      const frames = await extractFrames(src, times, framesDir(dir));
      return {
        info: {} as Record<string, unknown>,
        frames: frames.map((f) => f.path),
      };
    },
  };

  const edit = {
    async execute(input: { op: import("@cut/timeline").EditOp }) {
      const current = await loadTimeline(dir);
      if (!current) throw new Error("no project timeline found (run cut with a video first)");
      const updated = applyEdit(current, input.op);
      await saveTimeline(dir, updated);
      const summary =
        `${updated.tracks[0]?.clips.length ?? 0} clips, ` +
        `duration ${timelineDuration(updated).toFixed(1)}s`;
      console.log(renderTimelineGantt(updated));
      return { ...updated, summary };
    },
  } satisfies ToolImpl<{ op: import("@cut/timeline").EditOp }, unknown>;

  const doRender = {
    async execute(input: { draft?: boolean }) {
      const current = await loadTimeline(dir);
      if (!current) throw new Error("no project timeline found");
      const output = path.join(
        path.resolve(dir),
        input.draft ? "output_draft.mp4" : "output.mp4",
      );
      let lastLine = -1;
      await render(current, {
        output,
        ...(input.draft ? { draft: true } : {}),
        onProgress: (ratio) => {
          if (!process.stdout.isTTY && !process.env["CUT_FORCE_PROGRESS"]) return;
          const line = Math.floor(ratio * 20);
          if (line !== lastLine) {
            lastLine = line;
            console.log(`render ${progressBar(ratio)}`);
          }
        },
      });
      console.log(`\n${renderTimelineGantt(current)}\nrendered → ${output}`);
      void showFrame(output).catch(() => {});
      hooks.onRenderDone?.(output);
      return { output };
    },
  } satisfies ToolImpl<{ draft?: boolean }, { output: string }>;

  const bash = {
    async execute(input: { command: string }) {
      return new Promise<{ stdout: string; stderr: string; exitCode: number }>((resolve) => {
        execFile(
          "/bin/sh",
          ["-c", input.command],
          { cwd: path.resolve(dir), timeout: 120_000, maxBuffer: 8 * 1024 * 1024 },
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

  return { fetch, probe, edit, render: doRender, bash };
}
