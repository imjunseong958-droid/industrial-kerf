import { describe, expect, it } from "vitest";
import {
  BUNDLED_SCENARIOS,
  ScenarioSchema,
  evaluateFinalV1Result,
  evaluateFinalV2Result,
  evaluateScenario,
  evaluateScenarios,
  evaluateToolTrace,
  normalizeToolTrace,
  type EvalScenario,
} from "../src/index.js";

describe("deterministic eval corpus", () => {
  it("ships at least ten representative scenarios and all pass", () => {
    expect(BUNDLED_SCENARIOS.length).toBeGreaterThanOrEqual(10);
    const report = evaluateScenarios(BUNDLED_SCENARIOS);
    expect(report.total).toBe(BUNDLED_SCENARIOS.length);
    expect(report.passed).toBe(true);
    expect(report.scenarios.every((scenario) => scenario.failures.length === 0)).toBe(true);
  });

  it("normalizes started/finished events without an LLM or media runtime", () => {
    const trace = normalizeToolTrace([
      { type: "tool_call_started", id: "t1", name: "timeline", input: { include: "full" } },
      { type: "tool_call_finished", id: "t1", name: "timeline", output: { version: 1 }, status: "ok" },
    ]);
    expect(trace).toHaveLength(1);
    expect(trace[0]).toMatchObject({ tool: "timeline", input: { include: "full" }, output: { version: 1 }, status: "ok" });
  });

  it("flags an edit before inspection and accepted shell execution", () => {
    const traceResult = evaluateToolTrace([
      { tool: "edit", input: { op: "trim", clipId: "c1", newOut: 2 }, status: "ok" },
      { tool: "bash", input: { command: "echo unsafe" }, status: "ok", output: { exitCode: 0 } },
    ], { invariants: ["inspect-before-edit", "no-bash"] });
    expect(traceResult.passed).toBe(false);
    expect(traceResult.failures.map((failure) => failure.invariant)).toEqual([
      "inspect-before-edit",
      "no-bash",
    ]);
  });
});

describe("final v1/v2 result evaluators", () => {
  it("rejects malformed v1 and v2 documents", () => {
    const v1 = evaluateFinalV1Result({ version: 1, fps: 30, sources: [], tracks: [], overlays: [] }, { invariants: ["valid-timeline"] });
    const v2 = evaluateFinalV2Result({ project: { version: 2, fps: 30, sources: [], tracks: [] } }, { invariants: ["valid-timeline"] });
    expect(v1.passed).toBe(true);
    expect(v2.passed).toBe(true);

    const invalidV1 = evaluateFinalV1Result({ version: 1, fps: 0, sources: [], tracks: [], overlays: [] }, { invariants: ["valid-timeline"] });
    const invalidV2 = evaluateFinalV2Result({ project: { version: 2, fps: 30, sources: [], tracks: [{ id: "v", type: "video", clips: [{ id: "missing", sourceId: "none", in: 0, out: 1, timelineStart: 0 }] }] } }, { invariants: ["valid-timeline"] });
    expect(invalidV1.passed).toBe(false);
    expect(invalidV2.passed).toBe(false);
  });

  it("parses a scenario through the public runtime schema", () => {
    const source = BUNDLED_SCENARIOS[0];
    const parsed = ScenarioSchema.parse(source) as EvalScenario;
    expect(parsed.id).toBe("shorts-youtube-vtt-no-whisper");
    expect(evaluateScenario(parsed).passed).toBe(true);
  });
});
