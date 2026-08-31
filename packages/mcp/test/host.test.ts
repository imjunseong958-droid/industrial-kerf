import { describe, expect, it, afterEach, beforeEach } from "vitest";
import { vi } from "vitest";
import { mkdtemp, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import type { RenderJobManager } from "@cut/media";
import { createCutMcpProjectHost } from "../src/host.js";
import { createCutMcpRouter } from "../src/server.js";

function legacyTimeline() {
  return {
    version: 1 as const,
    fps: 30,
    sources: [{ id: "src1", path: "media/input.mp4", subs: "captions.vtt", duration: 10 }],
    tracks: [{ id: "v1", clips: [{ id: "c1", src: "src1", in: 0, out: 10 }] }],
    overlays: [{ id: "subs1", type: "subtitles", path: "captions.vtt" }],
  };
}

describe("project-scoped cut MCP host", () => {
  const dirs: string[] = [];
  beforeEach(() => {
    vi.stubEnv("HOME", tmpdir());
  });
  afterEach(async () => {
    await Promise.all(dirs.splice(0).map((dir) => rm(dir, { recursive: true, force: true })));
  });

  it("migrates v1 in memory, applies v2 commands, and persists atomically", async () => {
    const dir = await mkdtemp(path.join(tmpdir(), "cut-mcp-host-"));
    dirs.push(dir);
    await writeFile(
      path.join(dir, "project.cut.json"),
      JSON.stringify(legacyTimeline()),
      "utf8",
    );
    const host = createCutMcpProjectHost({ dir });
    const router = createCutMcpRouter(host.handlers);

    const inspected = await router.call("project.inspect", { include: "full" });
    expect(inspected.structuredContent).toMatchObject({
      ok: true,
      result: { sourceVersion: 1, normalizedVersion: 2, revision: 0 },
    });
    const snapshot = await host.history();
    snapshot.project.sources.push({ id: "outside", path: "/tmp/outside.mp4" });
    expect((await host.history()).project.sources.some((source) => source.id === "outside")).toBe(false);

    const applied = await router.call("timeline.apply", {
      expectedRevision: 0,
      command: {
        type: "update_clip",
        clipId: "c1",
        patch: { in: 2, out: 8 },
      },
    });
    expect(applied.structuredContent).toMatchObject({
      ok: true,
      result: { revision: 1, project: { version: 2 } },
    });
    expect(JSON.parse(await readFile(path.join(dir, "project.cut.json"), "utf8"))).toMatchObject({
      version: 2,
      tracks: [{ clips: [{ in: 2, out: 8 }] }],
    });

    const stale = await router.call("timeline.apply", {
      expectedRevision: 0,
      command: {
        type: "update_clip",
        clipId: "c1",
        patch: { in: 3 },
      },
    });
    expect(stale).toMatchObject({ isError: true });
    expect(stale.structuredContent).toMatchObject({
      ok: false,
      error: { code: "REVISION_CONFLICT", details: { actualRevision: 1 } },
    });

    const undone = await router.call("timeline.undo", { expectedRevision: 1 });
    expect(undone.structuredContent).toMatchObject({ ok: true, result: { revision: 2 } });
    const redone = await router.call("timeline.redo", { expectedRevision: 2 });
    expect(redone.structuredContent).toMatchObject({ ok: true, result: { revision: 3 } });
    await host.close();
  });

  it("reports validation and diffs without accepting filesystem paths", async () => {
    const dir = await mkdtemp(path.join(tmpdir(), "cut-mcp-host-"));
    dirs.push(dir);
    const host = createCutMcpProjectHost({ dir });
    const router = createCutMcpRouter(host.handlers);
    const missing = await router.call("project.validate", { strict: true });
    expect(missing.structuredContent).toMatchObject({ ok: true, result: { valid: false } });

    // A path-like field is not part of any handler schema; the router rejects
    // it before a host implementation can access anything outside dir.
    const rejected = await router.call("project.inspect", { path: "/etc/passwd" });
    expect(rejected.structuredContent).toMatchObject({ ok: false, error: { code: "INVALID_ARGUMENTS" } });
    await host.close();
  });

  it("includes explicit project duration and root gaps in summaries", async () => {
    const dir = await mkdtemp(path.join(tmpdir(), "cut-mcp-host-duration-"));
    dirs.push(dir);
    await writeFile(path.join(dir, "project.cut.json"), JSON.stringify({
      version: 2,
      fps: 30,
      duration: 30,
      sources: [],
      tracks: [],
      overlays: [],
      gaps: [{ id: "gap1", start: 10, duration: 40 }],
    }), "utf8");
    const host = createCutMcpProjectHost({ dir });
    const router = createCutMcpRouter(host.handlers);
    const inspected = await router.call("project.inspect", {});
    expect(inspected.structuredContent).toMatchObject({ ok: true, result: { duration: 50 } });
    await host.close();
  });

  it("normalizes legacy operation batches against each intermediate project", async () => {
    const dir = await mkdtemp(path.join(tmpdir(), "cut-mcp-host-batch-"));
    dirs.push(dir);
    await writeFile(path.join(dir, "project.cut.json"), JSON.stringify(legacyTimeline()), "utf8");
    const host = createCutMcpProjectHost({ dir });
    const router = createCutMcpRouter(host.handlers);
    const result = await router.call("timeline.apply", {
      expectedRevision: 0,
      ops: [
        { op: "add_clip", clip: { src: "src1", in: 0, out: 1 } },
        { op: "add_clip", clip: { src: "src1", in: 1, out: 2 } },
      ],
    });
    expect(result.structuredContent).toMatchObject({ ok: true, result: { revision: 1 } });
    const project = (result.structuredContent as { result: { project: { tracks: Array<{ clips: Array<{ id: string }> }> } } }).result.project;
    expect(project.tracks[0]!.clips.map((clip) => clip.id)).toEqual(["c1", "clip1", "clip2"]);
    await host.close();
  });

  it("runs project-scoped preview/full jobs through the media scheduler", async () => {
    const dir = await mkdtemp(path.join(tmpdir(), "cut-mcp-host-render-"));
    dirs.push(dir);
    await writeFile(path.join(dir, "project.cut.json"), JSON.stringify(legacyTimeline()), "utf8");
    const statuses = new Map<string, Record<string, unknown>>();
    const recordJob = (
      project: unknown,
      options: Record<string, unknown>,
      jobOptions: Record<string, unknown> = {},
    ): string => {
      const jobId = String(jobOptions.id);
      statuses.set(jobId, {
        id: jobId,
        status: "completed",
        state: "completed",
        progress: { ratio: 1 },
        createdAt: Date.now(),
        completedAt: Date.now(),
        metadata: jobOptions.metadata,
        project,
        options,
      });
      return jobId;
    };
    const startV1 = vi.fn((project: unknown, options: Record<string, unknown>, _callback: unknown, jobOptions: Record<string, unknown> = {}) =>
      recordJob(project, options, jobOptions));
    const startV2 = vi.fn((project: unknown, options: Record<string, unknown>, _callback: unknown, jobOptions: Record<string, unknown> = {}) =>
      recordJob(project, options, jobOptions));
    const renderManager = {
      startV1,
      startV2,
      status: (jobId: string) => statuses.get(jobId),
      cancel: vi.fn(() => true),
      result: vi.fn(async () => undefined),
    } as unknown as RenderJobManager;
    const host = createCutMcpProjectHost({ dir, renderManager });
    const router = createCutMcpRouter(host.handlers);

    expect(router.toolNames).toEqual(expect.arrayContaining([
      "render.preview",
      "render.start",
      "render.status",
      "render.cancel",
      "render.result",
    ]));

    const preview = await router.call("render.preview", {});
    expect(preview.structuredContent).toMatchObject({ ok: true, result: { revision: 0 } });
    const previewResult = (preview.structuredContent as { result: { jobId: string; output: string } }).result;
    expect(previewResult.output).toBe(path.join(dir, `${previewResult.jobId}.mp4`));
    const previewDone = await router.call("render.result", { jobId: previewResult.jobId });
    expect(previewDone.structuredContent).toMatchObject({
      ok: true,
      result: { jobId: previewResult.jobId, status: { status: "completed" }, output: previewResult.output },
    });

    const applied = await router.call("timeline.apply", {
      expectedRevision: 0,
      command: { type: "update_clip", clipId: "c1", patch: { in: 1, out: 9 } },
    });
    expect(applied.structuredContent).toMatchObject({ ok: true, result: { revision: 1 } });
    const full = await router.call("render.start", { draft: false });
    const fullResult = (full.structuredContent as { result: { jobId: string; output: string } }).result;
    const fullDone = await router.call("render.result", { jobId: fullResult.jobId });
    expect(fullDone.structuredContent).toMatchObject({ ok: true, result: { status: { status: "completed" } } });
    expect(startV1).not.toHaveBeenCalled();
    expect(startV2).toHaveBeenCalledWith(
      expect.objectContaining({ version: 2 }),
      expect.objectContaining({ draft: true, output: previewResult.output }),
      undefined,
      expect.objectContaining({ id: previewResult.jobId }),
    );
    const renderedPreview = startV2.mock.calls[0]![0] as {
      sources: Array<{ path: string; subs?: string }>;
      overlays: Array<{ path?: string }>;
    };
    expect(renderedPreview.sources[0]).toMatchObject({
      path: path.join(dir, "media/input.mp4"),
      subs: path.join(dir, "captions.vtt"),
    });
    expect(renderedPreview.overlays[0]?.path).toBe(path.join(dir, "captions.vtt"));
    expect(startV2).toHaveBeenCalledWith(
      expect.objectContaining({ version: 2 }),
      expect.objectContaining({ draft: false, output: fullResult.output }),
      undefined,
      expect.objectContaining({ id: fullResult.jobId }),
    );
    await host.close();
  });

  it("creates a v2 project on fetch, appends twice, and imports a local file", async () => {
    const dir = await mkdtemp(path.join(tmpdir(), "cut-mcp-host-media-"));
    dirs.push(dir);
    const mediaPath = path.join(dir, "local.mp4");
    await writeFile(mediaPath, "video");
    const fetchVideo = vi.fn(async (url: string) => {
      const name = url.endsWith("second") ? "second.mp4" : "first.mp4";
      const mediaPath = path.join(dir, name);
      await writeFile(mediaPath, "video");
      return { path: mediaPath, title: name };
    });
    const info = vi.fn().mockResolvedValue({
      duration: 10,
      width: 1920,
      height: 1080,
      hasAudio: true,
    });
    const host = createCutMcpProjectHost({ dir, fetchVideo, info });
    const router = createCutMcpRouter(host.handlers);

    const first = await router.call("media.fetch", { url: "https://example.com/first" });
    expect(first.structuredContent).toMatchObject({
      ok: true,
      result: { path: path.join(dir, "first.mp4"), sourceId: expect.stringMatching(/^source-/), revision: 1 },
    });
    const second = await router.call("media.fetch", { url: "https://example.com/second" });
    expect(second.structuredContent).toMatchObject({
      ok: true,
      result: { sourceId: expect.stringMatching(/^source-/), revision: 2 },
    });
    const inspected = await router.call("project.inspect", { include: "full" });
    const project = (inspected.structuredContent as { result: { project: { sources: unknown[] } } }).result.project;
    expect(project.sources).toHaveLength(2);

    const imported = await router.call("media.import", { path: "local.mp4" });
    expect(imported.structuredContent).toMatchObject({
      ok: true,
      result: { path: mediaPath, sourceId: expect.stringMatching(/^source-/), revision: 3 },
    });
    const duplicate = await router.call("media.import", { path: "local.mp4" });
    expect(duplicate.structuredContent).toMatchObject({
      ok: true,
      result: { sourceId: expect.stringMatching(/^source-/), revision: 3 },
    });
    await host.close();
  });

  it("returns JSON frame probes and advertises vision only when configured", async () => {
    const dir = await mkdtemp(path.join(tmpdir(), "cut-mcp-host-probe-"));
    dirs.push(dir);
    const mediaPath = path.join(dir, "clip.mp4");
    await writeFile(mediaPath, "video");
    const framePath = path.join(dir, ".cut", "frames", "frame_0.png");
    await mkdir(path.dirname(framePath), { recursive: true });
    await writeFile(framePath, Buffer.from("png-bytes"));
    const extractFrames = vi.fn().mockResolvedValue([{ time: 1.5, path: framePath }]);
    const info = vi.fn().mockResolvedValue({ duration: 10, hasAudio: true });
    const host = createCutMcpProjectHost({ dir, info, extractFrames });
    const router = createCutMcpRouter(host.handlers);
    expect(router.toolNames).not.toContain("vision.track_objects");

    const probed = await router.call("media.probe", {
      action: "frames",
      src: "clip.mp4",
      opts: { frames: 1 },
    });
    expect(probed.structuredContent).toMatchObject({
      ok: true,
      result: {
        action: "frames",
        frames: [{ time: 1.5, path: framePath, imageBase64: expect.any(String), mediaType: "image/png" }],
      },
    });

    const tracker = {
      track: vi.fn().mockResolvedValue({
        detections: [],
        metadata: { frameCount: 0 },
      }),
    };
    const visionHost = createCutMcpProjectHost({ dir, info, extractFrames, vision: tracker });
    const visionRouter = createCutMcpRouter(visionHost.handlers);
    expect(visionRouter.toolNames).toContain("vision.track_objects");
    await host.close();
    await visionHost.close();
  });

  it("persists legacy add_source and add_subtitles via granular commands", async () => {
    const dir = await mkdtemp(path.join(tmpdir(), "cut-mcp-host-legacy-source-"));
    dirs.push(dir);
    await writeFile(path.join(dir, "project.cut.json"), JSON.stringify(legacyTimeline()), "utf8");
    const host = createCutMcpProjectHost({ dir });
    const router = createCutMcpRouter(host.handlers);

    const addedSource = await router.call("timeline.apply", {
      expectedRevision: 0,
      op: {
        op: "add_source",
        source: { id: "extra", path: path.join(dir, "extra.mp4"), duration: 5 },
      },
    });
    expect(addedSource.structuredContent).toMatchObject({ ok: true, result: { revision: 1 } });

    const addedSubtitles = await router.call("timeline.apply", {
      expectedRevision: 1,
      op: { op: "add_subtitles", path: path.join(dir, "new-subs.vtt") },
    });
    expect(addedSubtitles.structuredContent).toMatchObject({ ok: true, result: { revision: 2 } });

    const persisted = JSON.parse(await readFile(path.join(dir, "project.cut.json"), "utf8"));
    expect(persisted.sources.some((source: { id: string }) => source.id === "extra")).toBe(true);
    expect(
      persisted.overlays.some(
        (overlay: { type?: string; path?: string }) =>
          overlay.type === "subtitles" && overlay.path === path.join(dir, "new-subs.vtt"),
      ),
    ).toBe(true);
    await host.close();
  });

  it("supports legacy split and remove_ranges on a migrated v1 project", async () => {
    const dir = await mkdtemp(path.join(tmpdir(), "cut-mcp-host-legacy-"));
    dirs.push(dir);
    await writeFile(path.join(dir, "project.cut.json"), JSON.stringify(legacyTimeline()), "utf8");
    const host = createCutMcpProjectHost({ dir });
    const router = createCutMcpRouter(host.handlers);

    const split = await router.call("timeline.apply", {
      expectedRevision: 0,
      op: { op: "split", clipId: "c1", at: 5 },
    });
    expect(split.structuredContent).toMatchObject({ ok: true, result: { revision: 1 } });
    const splitProject = (split.structuredContent as { result: { project: { tracks: Array<{ clips: Array<{ id: string }> }> } } }).result.project;
    expect(splitProject.tracks[0]!.clips.map((clip) => clip.id)).toEqual(["c1", "c1-b"]);

    const removed = await router.call("timeline.apply", {
      expectedRevision: 1,
      op: { op: "remove_ranges", ranges: [{ start: 1, end: 2 }] },
    });
    expect(removed.structuredContent).toMatchObject({ ok: true, result: { revision: 2 } });
    await host.close();
  });
});
