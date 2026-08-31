import { describe, expect, it, beforeEach, afterEach, vi } from "vitest";
import { mkdtemp, readFile, readdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { buildCutTools } from "../src/tools.js";
import { initialTimeline, loadProjectV2, loadTimeline, projectPath, saveProjectV2, saveTimeline } from "../src/state.js";
import { ProjectSchema, type Timeline } from "@cut/timeline";

vi.mock("@cut/fetcher", () => ({
  fetchVideo: vi.fn(),
}));
vi.mock("@cut/media", async () => {
  const actual = await vi.importActual<typeof import("@cut/media")>("@cut/media");
  return { ...actual, info: vi.fn(), extractFrames: vi.fn() };
});

const { fetchVideo } = await import("@cut/fetcher");
const { extractFrames, info } = await import("@cut/media");

function sampleTimeline(duration = 10): Timeline {
  return initialTimeline("src1", "/tmp/video.mp4", {
    duration,
    width: 1920,
    height: 1080,
    hasAudio: true,
  });
}

describe("timeline persistence and tool transactions", () => {
  let dir: string;

  beforeEach(async () => {
    dir = await mkdtemp(path.join(tmpdir(), "cut-reliability-"));
    vi.clearAllMocks();
  });

  afterEach(async () => {
    await rm(dir, { recursive: true, force: true });
  });

  it("rejects invalid edit input without changing the project file", async () => {
    await saveTimeline(dir, sampleTimeline());
    const before = await readFile(projectPath(dir), "utf8");
    const tools = buildCutTools({ dir });

    await expect(
      tools.project_edit!.execute({
        op: { op: "trim", clipId: "c1", newOut: 5 },
        ops: [{ op: "trim", clipId: "c1", newOut: 4 }],
      } as never),
    ).rejects.toThrow();
    expect(await readFile(projectPath(dir), "utf8")).toBe(before);
  });

  it("applies edit batches atomically when a later operation fails", async () => {
    await saveTimeline(dir, sampleTimeline());
    const tools = buildCutTools({ dir });

    await expect(
      tools.project_edit!.execute({
        ops: [
          { op: "trim", clipId: "c1", newOut: 5 },
          { op: "remove_clip", clipId: "missing" },
        ],
      }),
    ).rejects.toThrow(/clip not found/);
    const project = await loadProjectV2(dir);
    expect(project!.tracks[0]!.clips[0]!.out).toBe(10);
  });

  it("exposes the MCP timeline query shape", async () => {
    await saveTimeline(dir, sampleTimeline());
    const result = await buildCutTools({ dir }).timeline.execute({});
    expect(result).toMatchObject({
      revision: 0,
      tracks: [
        {
          id: "v1",
          clips: [
            {
              id: "c1",
              sourceId: "src1",
              timelineStart: 0,
              duration: 10,
              in: 0,
              out: 10,
            },
          ],
        },
      ],
    });
  });

  it("returns extracted PNG bytes with exact frame time/path metadata", async () => {
    const firstPath = path.join(dir, "frame-a.png");
    const secondPath = path.join(dir, "frame-b.png");
    const firstBytes = Buffer.from([0, 1, 2, 3, 4]);
    const secondBytes = Buffer.from([9, 8, 7, 6]);
    await writeFile(firstPath, firstBytes);
    await writeFile(secondPath, secondBytes);
    vi.mocked(info).mockResolvedValue({ duration: 4 });
    vi.mocked(extractFrames).mockResolvedValue([
      { time: 0.5, path: firstPath },
      { time: 2.5, path: secondPath },
    ]);

    const result = await buildCutTools({ dir }).probe.execute({
      action: "frames",
      src: "video.mp4",
      opts: { frames: 2 },
    });
    expect(result.type).toBe("multimodal_tool_result");
    const multimodal = result as Extract<typeof result, { type: "multimodal_tool_result" }>;
    expect(multimodal.metadata).toEqual([
      { time: 0.5, path: firstPath },
      { time: 2.5, path: secondPath },
    ]);
    expect(multimodal.images).toHaveLength(2);
    expect(multimodal.images[0]!.mediaType).toBe("image/png");
    expect(Buffer.from(multimodal.images[0]!.data, "base64")).toEqual(firstBytes);
    expect(Buffer.from(multimodal.images[1]!.data, "base64")).toEqual(secondBytes);
  });

  it("rejects frame requests above the eight-image session limit", async () => {
    await expect(
      buildCutTools({ dir }).probe.execute({
        action: "frames",
        src: "video.mp4",
        opts: { frames: 9 },
      }),
    ).rejects.toThrow();
  });

  it("returns per-track timelineStart values in the MCP query shape", async () => {
    const timeline = sampleTimeline(10);
    timeline.tracks.push({
      id: "v2",
      clips: [{ id: "c2", src: "src1", in: 0, out: 2 }],
    });
    await saveTimeline(dir, timeline);
    const result = await buildCutTools({ dir }).timeline.execute({});
    const clips = result.tracks.flatMap((track: { clips: Array<{ id: string; timelineStart: number; duration: number }> }) => track.clips);
    expect(clips.map((clip) => [clip.id, clip.timelineStart, clip.duration])).toEqual([
      ["c1", 0, 10],
      ["c2", 0, 2],
    ]);
  });

  it("reports corruption instead of treating it as a missing project", async () => {
    await writeFile(projectPath(dir), "{ this is not json", "utf8");
    await expect(loadTimeline(dir)).rejects.toThrow(/malformed JSON/);
  });

  it("writes via a same-directory atomic rename and leaves no temp files", async () => {
    await saveTimeline(dir, sampleTimeline());
    const names = await readdir(dir);
    expect(names).toContain("project.cut.json");
    expect(names.filter((name) => name.includes("project.cut.json.") && name.endsWith(".tmp"))).toEqual([]);
    expect(await loadTimeline(dir)).toMatchObject({ version: 1 });
  });

  it("appends each fetched media result as a source and full-length clip", async () => {
    const firstPath = path.join(dir, "first.mp4");
    const secondPath = path.join(dir, "second.mp4");
    await Promise.all([writeFile(firstPath, "first"), writeFile(secondPath, "second")]);
    vi.mocked(fetchVideo)
      .mockResolvedValueOnce({ path: firstPath, subsPath: "/tmp/first.vtt" })
      .mockResolvedValueOnce({ path: secondPath, subsPath: "/tmp/second.vtt" });
    vi.mocked(info)
      .mockResolvedValueOnce({ duration: 4, width: 1280, height: 720, hasAudio: true })
      .mockResolvedValueOnce({ duration: 7, width: 1920, height: 1080, hasAudio: false });
    const tools = buildCutTools({ dir });

    await tools.fetch.execute({ url: "https://example.test/first" });
    await tools.fetch.execute({ url: "https://example.test/second" });

    const project = (await loadProjectV2(dir))!;
    expect(project.version).toBe(2);
    expect(project.sources).toHaveLength(2);
    expect(project.sources.map((source) => [source.path, source.subs, source.duration])).toEqual([
      [firstPath, "/tmp/first.vtt", 4],
      [secondPath, "/tmp/second.vtt", 7],
    ]);
    expect(project.tracks[0]!.clips.map((clip) => [clip.in, clip.out])).toEqual([
      [0, 4],
      [0, 7],
    ]);
    expect(project.tracks[0]!.clips.map((clip) => clip.sourceId)).toEqual(
      project.sources.map((source) => source.id),
    );
  });

  it("appends the first fetched source at zero in an empty v2 project", async () => {
    await saveProjectV2(dir, ProjectSchema.parse({
      version: 2,
      fps: 30,
      sources: [],
      tracks: [],
      overlays: [],
      gaps: [],
      transitions: [],
      effects: [],
      markers: [],
    }));
    const firstPath = path.join(dir, "first.mp4");
    await writeFile(firstPath, "first");
    vi.mocked(fetchVideo).mockResolvedValue({ path: firstPath });
    vi.mocked(info).mockResolvedValue({ duration: 4, width: 1280, height: 720, hasAudio: true });

    await buildCutTools({ dir }).fetch.execute({ url: "https://example.test/first" });

    const project = (await loadProjectV2(dir))!;
    expect(project.tracks[0]!.clips[0]).toMatchObject({
      sourceId: project.sources[0]!.id,
      timelineStart: 0,
      in: 0,
      out: 4,
    });
  });
});
