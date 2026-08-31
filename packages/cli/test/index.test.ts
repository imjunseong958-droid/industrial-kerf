import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { renderTimelineGantt, progressBar } from "../src/gantt.js";
import { buildCutTools } from "../src/tools.js";
import { initialTimeline, loadProjectV2, saveTimeline } from "../src/state.js";
import type { Timeline } from "@cut/timeline";

function sampleTimeline(duration = 10): Timeline {
  return initialTimeline("src1", "video.mp4", { duration });
}

describe("renderTimelineGantt", () => {
  it("renders a single-clip gantt with total", () => {
    const line = renderTimelineGantt(sampleTimeline(10));
    expect(line).toContain("v1");
    expect(line).toContain("▓");
    expect(line).toContain("total 10.0s");
  });

  it("scales multiple clips to ~60 chars and shows boundaries", () => {
    const tl = sampleTimeline(20);
    tl.tracks[0]!.clips.push({ id: "c2", src: "src1", in: 20, out: 28 });
    const line = renderTimelineGantt(tl);
    expect(line).toContain("|");
    expect(line).toContain("00.0-20.0");
    expect(line).toContain("20.0-28.0");
  });

  it("handles empty timeline", () => {
    expect(renderTimelineGantt({ ...sampleTimeline(), tracks: [] })).toBe("(empty timeline)");
  });

  it("progressBar clamps", () => {
    expect(progressBar(-1)).toContain("0%");
    expect(progressBar(2)).toContain("100%");
  });
});

// A mock AgentSession-like helper exercising the tool registry directly.
describe("tool wiring state logic", () => {
  let dir: string;

  beforeEach(async () => {
    dir = await mkdtemp(path.join(tmpdir(), "cut-cli-test-"));
  });
  afterEach(async () => {
    await rm(dir, { recursive: true, force: true });
  });

  async function seed(): Promise<void> {
    await saveTimeline(dir, sampleTimeline(10));
  }

  it("project_edit round-trips trim to disk as v2", async () => {
    await seed();
    const tools = buildCutTools({ dir });
    expect(tools.edit).toBeUndefined();
    await tools.project_edit!.execute({
      op: { op: "trim", clipId: "c1", newOut: 5 },
    });

    const onDisk = await loadProjectV2(dir);
    expect(onDisk!.version).toBe(2);
    expect(onDisk!.tracks[0]!.clips[0]!.out).toBe(5);
  });

  it("split creates two clips persisted across calls", async () => {
    await seed();
    const tools = buildCutTools({ dir });
    await tools.project_edit!.execute({ op: { op: "split", clipId: "c1", at: 4 } });
    const onDisk = await loadProjectV2(dir);
    expect(onDisk!.tracks[0]!.clips.map((c) => c.id)).toEqual(["c1", "c1-b"]);
    expect(onDisk!.tracks[0]!.clips[0]!.out).toBe(4);
    expect(onDisk!.tracks[0]!.clips[1]!.in).toBe(4);
  });

  it("set_speed persists", async () => {
    await seed();
    const tools = buildCutTools({ dir });
    await tools.project_edit!.execute({ op: { op: "set_speed", clipId: "c1", speed: 2 } });
    expect((await loadProjectV2(dir))!.tracks[0]!.clips[0]!.speed).toBe(2);
  });

  it("project_edit rejects an edit against the empty canonical project", async () => {
    const tools = buildCutTools({ dir });
    await expect(
      tools.project_edit!.execute({ op: { op: "trim", clipId: "c1", newOut: 5 } }),
    ).rejects.toThrow(/clip not found/);
  });

  it("bash runs in the session dir", async () => {
    await seed();
    const tools = buildCutTools({ dir, enableBash: true });
    const res = await tools.bash!.execute({ command: "pwd" });
    expect(res.exitCode).toBe(0);
    expect(path.basename(res.stdout.trim())).toBe(path.basename(dir));
  });

  it("does not expose bash unless explicitly enabled", async () => {
    await seed();
    // User-level vision configuration must not change the core tool catalog
    // asserted by this test.
    const tools = buildCutTools({ dir, vision: {} });
    expect(tools.bash).toBeUndefined();
    expect(Object.keys(tools).sort()).toEqual([
      "fetch",
      "probe",
      "project_edit",
      "render",
      "skill_read",
      "timeline",
      "timeline_redo",
      "timeline_undo",
    ]);
    expect(tools.timeline_query).toBeUndefined();
    expect(tools.timeline_apply).toBeUndefined();
  });

  it("exposes bash when the explicit environment opt-in is set", async () => {
    await seed();
    const previous = process.env["CUT_ENABLE_BASH"];
    process.env["CUT_ENABLE_BASH"] = "1";
    try {
      expect(buildCutTools({ dir }).bash).toBeDefined();
    } finally {
      if (previous === undefined) delete process.env["CUT_ENABLE_BASH"];
      else process.env["CUT_ENABLE_BASH"] = previous;
    }
  });

  it("saveTimeline writes valid JSON at projectPath", async () => {
    const tl = sampleTimeline();
    await saveTimeline(dir, tl);
    const raw = JSON.parse(await readFile(path.join(dir, "project.cut.json"), "utf-8"));
    expect(raw.version).toBe(1);
    expect(raw.sources[0].id).toBe("src1");
  });
});
