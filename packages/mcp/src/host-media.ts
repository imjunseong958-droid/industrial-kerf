import { readFile } from "node:fs/promises";
import path from "node:path";
import { fetchVideo } from "@cut/fetcher";
import {
  detectScenes,
  extractFrames,
  info,
  silenceDetect,
  type ExtractedFrame,
  type ProbeInfo,
} from "@cut/media";
import {
  type Project,
  type ProjectHistory,
} from "@cut/timeline";
import type {
  CutMcpHandler,
  CutMcpMediaHandlers,
  MediaFetchInput,
  MediaImportInput,
  MediaProbeInput,
} from "./server.js";
import { resolveImportPath, resolveMediaSrc } from "./media-path.js";
import type { RuntimeClient } from "./runtime-client.js";

export interface MediaHandlerDeps {
  dir: string;
  cookies?: string;
  fetchVideo?: (
    url: string,
    outDir: string,
    opts?: { cookies?: string },
  ) => Promise<{ path: string; subsPath?: string; title?: string }>;
  info?: (src: string) => Promise<ProbeInfo>;
  silenceDetect?: typeof silenceDetect;
  detectScenes?: typeof detectScenes;
  extractFrames?: typeof extractFrames;
  ensureProject: () => Promise<ProjectHistory>;
  /** The shared Rust runtime is the only mutation authority. */
  runtime: RuntimeClient;
}

function framesDir(dir: string): string {
  return path.join(path.resolve(dir), ".cut", "frames");
}

export function createMediaHandlers(deps: MediaHandlerDeps): CutMcpMediaHandlers {
  const download = deps.fetchVideo ?? fetchVideo;
  const probeInfo = deps.info ?? info;
  const probeSilence = deps.silenceDetect ?? silenceDetect;
  const probeScenes = deps.detectScenes ?? detectScenes;
  const probeFrames = deps.extractFrames ?? extractFrames;

  const fetch: CutMcpHandler<MediaFetchInput> = async (input) => {
    // Downloading is an external adapter concern; committing the downloaded
    // artifact is still one canonical runtime transaction.
    await deps.ensureProject();
    const result = await download(input.url, deps.dir, {
      ...((input.cookies ?? deps.cookies) ? { cookies: input.cookies ?? deps.cookies } : {}),
    });
    const probed = await probeInfo(result.path);
    const current = await deps.runtime.getState();
    const requestValue = input as unknown as Record<string, unknown>;
    const requestKey = typeof requestValue.idempotencyKey === "string"
      ? requestValue.idempotencyKey
      : `mcp-fetch-${input.url}`;
    const imported = await deps.runtime.importMedia(
      "mcp",
      current.revision,
      requestKey,
      result.path,
      {
        metadata: {
          ...probed,
          ...(result.subsPath ? { subs: result.subsPath } : {}),
        },
      },
    );
    return {
      path: result.path,
      ...(result.subsPath ? { subsPath: result.subsPath } : {}),
      ...(result.title ? { title: result.title } : {}),
      sourceId: imported.sourceId,
      clipId: imported.clipId,
      revision: imported.state.revision,
    };
  };

  const importMedia: CutMcpHandler<MediaImportInput> = async (input) => {
    await deps.ensureProject();
    const resolved = resolveImportPath(deps.dir, input.path);
    const current = await deps.runtime.getState();
    const existing = current.project.sources.find((source) => {
      const sourcePath = path.isAbsolute(source.path)
        ? path.resolve(source.path)
        : path.resolve(deps.dir, source.path);
      return sourcePath === resolved;
    });
    if (existing) {
      const existingClip = current.project.tracks
        .flatMap((track) => track.clips)
        .find((clip) => clip.sourceId === existing.id);
      return {
        path: resolved,
        sourceId: existing.id,
        clipId: existingClip?.id ?? "",
        revision: current.revision,
      };
    }
    const requestValue = input as unknown as Record<string, unknown>;
    const requestKey = typeof requestValue.idempotencyKey === "string"
      ? requestValue.idempotencyKey
      : `mcp-import-${resolved}`;
    const result = await deps.runtime.importMedia("mcp", current.revision, requestKey, resolved, {
      ...(input.metadata ? { metadata: input.metadata } : {}),
      ...(input.overlay ? { overlay: input.overlay } : {}),
    });
    return { path: resolved, sourceId: result.sourceId, clipId: result.clipId, revision: result.state.revision };
  };

  const probe: CutMcpHandler<MediaProbeInput> = async (input, context) => {
    let project: Project | undefined;
    try {
      project = (await deps.ensureProject()).project;
    } catch {
      project = undefined;
    }
    const src = resolveMediaSrc(deps.dir, input.src, project);
    if (input.action === "info") {
      return { action: input.action, info: await probeInfo(src) };
    }
    if (input.action === "silence") {
      const ranges = await probeSilence(src, {
        ...(input.opts?.noiseDb !== undefined ? { thresholdDb: input.opts.noiseDb } : {}),
        ...(input.opts?.minDuration !== undefined ? { minDurationSec: input.opts.minDuration } : {}),
      });
      return {
        action: input.action,
        ranges: ranges.map((range) => ({ start: range.start, end: range.end })),
      };
    }
    if (input.action === "scenes") {
      const ranges = await probeScenes(src);
      return {
        action: input.action,
        ranges: ranges.map((range) => ({ start: range.start, end: range.end })),
      };
    }
    const n = input.opts?.frames ?? 8;
    const meta = await probeInfo(src);
    const duration = meta.duration ?? 60;
    const times = Array.from({ length: n }, (_, index) => (duration * index) / n + duration / (2 * n));
    const frames: ExtractedFrame[] = await probeFrames(src, times, framesDir(deps.dir));
    const serialized = await Promise.all(
      frames.map(async ({ time, path: framePath }) => ({
        time,
        path: framePath,
        imageBase64: (await readFile(framePath)).toString("base64"),
        mediaType: "image/png" as const,
      })),
    );
    const text = serialized.map(({ time, path: framePath }) => `Frame at ${time}s: ${framePath}`).join("\n");
    void context;
    return {
      action: "frames" as const,
      frames: serialized,
      text,
    };
  };

  return { fetch, import: importMedia, probe };
}
