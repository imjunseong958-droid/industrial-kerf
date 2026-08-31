import { describe, expect, it } from "vitest";
import {
  InterchangeAbortError,
  InterchangeConfigError,
  InterchangeFormatError,
  InterchangeTimeoutError,
  OpenTimelineIOAdapter,
  createOpenTimelineIOAdapter,
  normalizeAdapter,
  inspectProjectLosses,
} from "../src/index.js";
import type { Project } from "@cut/timeline";
import type { ProcessInvocation, ProcessResult } from "../src/index.js";

const PROJECT = {
  version: 2 as const,
  id: "project",
  fps: 24,
  width: 1920,
  height: 1080,
  sources: [{ id: "source", path: "/media/input.mov", duration: 10, width: 1920, height: 1080, hasAudio: true }],
  tracks: [
    {
      id: "video",
      type: "video" as const,
      clips: [
        {
          id: "clip",
          sourceId: "source",
          in: 1,
          out: 3,
          timelineStart: 2,
          duration: 2,
          crop: { x: 0, y: 0, width: 1, height: 1 },
          volume: 0.8,
          opacity: 0.9,
          speed: 2,
          blend: "normal",
          effects: [{ type: "blur" }],
          keyframes: [{ property: "opacity", time: 0, value: 1 }],
        },
      ],
      gaps: [{ id: "gap", start: 0, duration: 2 }],
    },
  ],
  overlays: [],
  gaps: [],
  transitions: [],
  effects: [],
  markers: [{ id: "marker", time: 2, label: "cut", color: "red" }],
};

function ok(stdout: string): ProcessResult {
  return { stdout, stderr: "", exitCode: 0 };
}

describe("OTIO adapter validation", () => {
  it("normalizes only friendly native format aliases", () => {
    expect(normalizeAdapter("otio")).toBe("otio_json");
    expect(normalizeAdapter(undefined, "/tmp/timeline.otioz")).toBe("otioz");
    expect(() => normalizeAdapter("fcp xml")).toThrow(InterchangeFormatError);
  });

  it("reports explicit placement overlaps instead of silently shifting them", () => {
    const overlap = {
      ...PROJECT,
      tracks: [{
        ...PROJECT.tracks[0],
        clips: [
          PROJECT.tracks[0]?.clips[0],
          { ...PROJECT.tracks[0]?.clips[0], id: "overlap", timelineStart: 1 },
        ],
      }],
    } as unknown as Project;
    const report = inspectProjectLosses(overlap, "export");
    const placement = report.losses.find((entry) => entry.code === "placement_overlap");
    expect(placement?.path).toContain("timelineStart");
    expect(placement?.details).toMatchObject({ requestedStart: 1, effectiveStart: 2 });
  });

  it("rejects invalid process configuration", () => {
    expect(() => new OpenTimelineIOAdapter({ timeoutMs: 0 })).toThrow(InterchangeConfigError);
    expect(() => new OpenTimelineIOAdapter({ pythonExecutable: "python\0" })).toThrow(InterchangeConfigError);
  });
});

describe("OpenTimelineIOAdapter", () => {
  it("imports a fake bridge project and reports unsupported v2 leaves", async () => {
    const calls: ProcessInvocation[] = [];
    const adapter = createOpenTimelineIOAdapter(
      { pythonExecutable: "/opt/python", bridgeScript: "/tmp/bridge.py" },
      {
        runner: async (invocation) => {
          calls.push(invocation);
          return ok(JSON.stringify({ ok: true, project: PROJECT, report: { warnings: ["adapter warning"] } }));
        },
      },
    );

    const result = await adapter.import("/tmp/timeline.otio", { adapter: "otio_json" });
    expect(calls[0]?.command).toBe("/opt/python");
    expect(calls[0]?.args).toEqual(["/tmp/bridge.py", "import", "--input", "/tmp/timeline.otio", "--adapter", "otio_json"]);
    expect(result.project.tracks[0]?.clips[0]?.timelineStart).toBe(2);
    expect(result.report.warnings.some((entry) => entry.code === "adapter_limitation")).toBe(true);
    expect(result.report.losses.map((entry) => entry.code)).toEqual(
      expect.arrayContaining(["effects_not_supported", "keyframes_not_supported", "blend_not_supported"]),
    );
  });

  it("exports v2 project JSON through stdin and infers otio_json for .otio", async () => {
    let invocation: ProcessInvocation | undefined;
    const adapter = createOpenTimelineIOAdapter({ bridgeScript: "/tmp/bridge.py" }, {
      runner: async (value) => {
        invocation = value;
        return ok(JSON.stringify({ ok: true, report: { issues: [] } }));
      },
    });

    const result = await adapter.export(PROJECT, "/tmp/timeline.otio");
    expect(result.adapter).toBe("otio_json");
    expect(invocation?.args).toEqual(["/tmp/bridge.py", "export", "--output", "/tmp/timeline.otio", "--adapter", "otio_json"]);
    expect(JSON.parse(invocation?.stdin ?? "{}").project.version).toBe(2);
    expect(result.report.losses.some((entry) => entry.code === "effects_not_supported")).toBe(true);
  });

  it("enforces timeout even when a fake runner ignores abort", async () => {
    const adapter = createOpenTimelineIOAdapter({}, { runner: async () => new Promise<ProcessResult>(() => undefined) });
    await expect(adapter.import("timeline.otio", { timeoutMs: 10 })).rejects.toBeInstanceOf(InterchangeTimeoutError);
  });

  it("propagates caller abort", async () => {
    const controller = new AbortController();
    const adapter = createOpenTimelineIOAdapter({}, { runner: async () => new Promise<ProcessResult>(() => undefined) });
    const pending = adapter.import("timeline.otio", { signal: controller.signal, timeoutMs: 10_000 });
    controller.abort();
    await expect(pending).rejects.toBeInstanceOf(InterchangeAbortError);
  });
});
