import { describe, expect, it } from "vitest";
import {
  buildCropPlan,
  buildTrackingPlan,
  planCropForAspectRatio,
  planTracking,
} from "../src/index.js";
import type { NormalizedDetection } from "../src/index.js";

function detection(
  overrides: Partial<NormalizedDetection> & Pick<NormalizedDetection, "trackId" | "class" | "frame" | "time" | "bbox">,
): NormalizedDetection {
  return {
    confidence: 0.9,
    frameIndex: overrides.frame,
    timeSeconds: overrides.time,
    ...overrides,
  };
}

describe("tracking planning", () => {
  it("keeps cam, chat, and content segments/keyframes separate", () => {
    const plan = buildTrackingPlan([
      detection({ trackId: 4, class: "chat", frame: 0, time: 0, bbox: { x: 0.55, y: 0.1, width: 0.3, height: 0.3 } }),
      detection({ trackId: 1, class: "cam", frame: 0, time: 0, bbox: { x: 0.1, y: 0.2, width: 0.15, height: 0.2 } }),
      detection({ trackId: 7, class: "content", frame: 0, time: 0, bbox: { x: 0.2, y: 0.25, width: 0.45, height: 0.5 } }),
      detection({ trackId: 1, class: "cam", frame: 1, time: 0.1, bbox: { x: 0.11, y: 0.2, width: 0.15, height: 0.2 } }),
      detection({ trackId: 4, class: "chat", frame: 1, time: 0.1, bbox: { x: 0.55, y: 0.1, width: 0.3, height: 0.3 } }),
      detection({ trackId: 7, class: "content", frame: 1, time: 0.1, bbox: { x: 0.2, y: 0.25, width: 0.45, height: 0.5 } }),
    ]);

    expect(plan.byClass.cam).toHaveLength(1);
    expect(plan.byClass.chat).toHaveLength(1);
    expect(plan.byClass.content).toHaveLength(1);
    expect(plan.keyframesByClass.cam.every((keyframe) => keyframe.class === "cam")).toBe(true);
    expect(plan.keyframesByClass.chat.every((keyframe) => keyframe.class === "chat")).toBe(true);
    expect(plan.keyframesByClass.content.every((keyframe) => keyframe.class === "content")).toBe(true);
    expect(plan.byClass.cam[0]?.keyframes[0]?.bbox).toEqual({ x: 0.1, y: 0.2, width: 0.15, height: 0.2 });
  });

  it("filters low confidence detections and splits tracks after max gap", () => {
    const plan = buildTrackingPlan([
      detection({ trackId: 1, class: "cam", frame: 0, time: 0, bbox: { x: 0, y: 0, width: 0.1, height: 0.1 } }),
      detection({ trackId: 1, class: "cam", frame: 1, time: 0.1, confidence: 0.1, bbox: { x: 0.1, y: 0, width: 0.1, height: 0.1 } }),
      detection({ trackId: 1, class: "cam", frame: 8, time: 0.8, bbox: { x: 0.2, y: 0, width: 0.1, height: 0.1 } }),
    ], { minConfidence: 0.5, maxGapSeconds: 0.2 });

    expect(plan.byClass.cam).toHaveLength(2);
    expect(plan.byClass.cam.map((segment) => segment.keyframes.length)).toEqual([1, 1]);
  });

  it("smooths bbox jitter deterministically within each segment", () => {
    const input = [
      detection({ trackId: 1, class: "cam", frame: 0, time: 0, bbox: { x: 0.1, y: 0.2, width: 0.2, height: 0.2 } }),
      detection({ trackId: 1, class: "cam", frame: 1, time: 0.1, bbox: { x: 0.3, y: 0.2, width: 0.2, height: 0.2 } }),
    ];
    const first = buildTrackingPlan(input, { smoothingFactor: 0.5 });
    const second = buildTrackingPlan(input, { smoothingFactor: 0.5 });
    expect(first).toEqual(second);
    expect(first.keyframes[1]?.bbox.x).toBe(0.2);
    expect(buildTrackingPlan(input, { smoothingFactor: 1 }).keyframes[1]?.bbox.x).toBe(0.3);
  });

  it("accepts a normalized result object and keeps empty classes available", () => {
    const plan = planTracking({
      source: "input.mp4",
      model: "custom.pt",
      targets: ["cam", "chat", "content"],
      frameCount: 1,
      detections: [detection({ trackId: 2, class: "cam", frame: 0, time: 0, bbox: { x: 0.1, y: 0.1, width: 0.2, height: 0.2 } })],
    });
    expect(plan.byClass.cam).toHaveLength(1);
    expect(plan.byClass.chat).toHaveLength(0);
    expect(plan.byClass.content).toHaveLength(0);
  });
});

describe("crop planning", () => {
  it("includes only content when that is the sole selected region", () => {
    const result = planCropForAspectRatio({
      aspectRatio: 16 / 9,
      sourceAspectRatio: 1,
      content: { x: 0.2, y: 0.25, width: 0.45, height: 0.5 },
      padding: 0.02,
    });

    expect(result.mode).toBe("content");
    expect(result.included).toEqual({ cam: false, chat: false, content: true });
    expect(result.crop.x).toBeGreaterThanOrEqual(0);
    expect(result.crop.y).toBeGreaterThanOrEqual(0);
    expect(result.crop.x + result.crop.width).toBeLessThanOrEqual(1);
    expect(result.crop.y + result.crop.height).toBeLessThanOrEqual(1);
  });

  it("includes cam and content when both are selected", () => {
    const result = planCropForAspectRatio({
      aspectRatio: 16 / 9,
      sourceAspectRatio: 1,
      cam: { x: 0.1, y: 0.35, width: 0.15, height: 0.2 },
      content: { x: 0.2, y: 0.25, width: 0.45, height: 0.5 },
      padding: 0.02,
    });

    expect(result.mode).toBe("cam_content");
    expect(result.included).toEqual({ cam: true, chat: false, content: true });
    expect(result.crop.x).toBeGreaterThanOrEqual(0);
    expect(result.crop.y).toBeGreaterThanOrEqual(0);
    expect(result.crop.x + result.crop.width).toBeLessThanOrEqual(1);
    expect(result.crop.y + result.crop.height).toBeLessThanOrEqual(1);
  });

  it("includes chat and content when both are selected", () => {
    const result = planCropForAspectRatio({
      aspectRatio: 16 / 9,
      sourceAspectRatio: 1,
      chat: { x: 0.55, y: 0.25, width: 0.3, height: 0.35 },
      content: { x: 0.2, y: 0.25, width: 0.45, height: 0.5 },
      padding: 0.02,
    });

    expect(result.mode).toBe("chat_content");
    expect(result.included).toEqual({ cam: false, chat: true, content: true });
    expect(result.crop.x).toBeGreaterThanOrEqual(0);
    expect(result.crop.y).toBeGreaterThanOrEqual(0);
    expect(result.crop.x + result.crop.width).toBeLessThanOrEqual(1);
    expect(result.crop.y + result.crop.height).toBeLessThanOrEqual(1);
  });

  it("includes all three classes when cam, chat, and content are selected", () => {
    const result = planCropForAspectRatio({
      aspectRatio: 16 / 9,
      sourceAspectRatio: 1,
      cam: { x: 0.1, y: 0.35, width: 0.15, height: 0.2 },
      chat: { x: 0.55, y: 0.25, width: 0.3, height: 0.35 },
      content: { x: 0.2, y: 0.25, width: 0.45, height: 0.5 },
      padding: 0.02,
    });

    expect(result.mode).toBe("all");
    expect(result.included).toEqual({ cam: true, chat: true, content: true });
    expect(result.crop.x).toBeGreaterThanOrEqual(0);
    expect(result.crop.y).toBeGreaterThanOrEqual(0);
    expect(result.crop.x + result.crop.width).toBeLessThanOrEqual(1);
    expect(result.crop.y + result.crop.height).toBeLessThanOrEqual(1);
    expect((result.crop.width * result.sourceAspectRatio) / result.crop.height).toBeCloseTo(16 / 9, 10);
  });

  it("tries to include both selected regions at the requested aspect ratio", () => {
    const result = planCropForAspectRatio({
      aspectRatio: 16 / 9,
      sourceAspectRatio: 1,
      cam: { x: 0.1, y: 0.35, width: 0.15, height: 0.2 },
      chat: { x: 0.55, y: 0.25, width: 0.3, height: 0.35 },
      padding: 0.02,
    });

    expect(result.mode).toBe("cam_chat");
    expect(result.included).toEqual({ cam: true, chat: true, content: false });
    expect(result.crop.x).toBeGreaterThanOrEqual(0);
    expect(result.crop.y).toBeGreaterThanOrEqual(0);
    expect(result.crop.x + result.crop.width).toBeLessThanOrEqual(1);
    expect(result.crop.y + result.crop.height).toBeLessThanOrEqual(1);
    expect((result.crop.width * result.sourceAspectRatio) / result.crop.height).toBeCloseTo(16 / 9, 10);
  });

  it("uses a centered aspect-ratio fallback when a class is missing", () => {
    const result = buildCropPlan({ aspectRatio: 1 });
    expect(result.mode).toBe("center");
    expect(result.included).toEqual({ cam: false, chat: false, content: false });
    expect(result.crop).toEqual({ x: 0, y: 0, width: 1, height: 1 });
  });

  it("accounts for source pixels when converting a 16:9 source to 9:16", () => {
    const result = planCropForAspectRatio({
      aspectRatio: 9 / 16,
      sourceAspectRatio: 16 / 9,
    });
    expect(result.crop.height).toBe(1);
    expect(result.crop.width).toBeCloseTo(0.31640625, 10);
    expect((result.crop.width * result.sourceAspectRatio) / result.crop.height).toBeCloseTo(9 / 16, 10);
  });

  it("anchors to the available class when the other classes are missing", () => {
    const result = buildCropPlan({
      aspectRatio: 1,
      cam: { x: 0.8, y: 0.8, width: 0.1, height: 0.1 },
    });
    expect(result.mode).toBe("cam");
    expect(result.included).toEqual({ cam: true, chat: false, content: false });
    expect(result.crop.x + result.crop.width).toBeLessThanOrEqual(1);
    expect(result.crop.y + result.crop.height).toBeLessThanOrEqual(1);
  });

  it("clamps out-of-frame targets and is deterministic", () => {
    const request = {
      aspectRatio: 9 / 16,
      cam: { x: 0.9, y: 0.85, width: 0.3, height: 0.3 },
    } as const;
    const first = buildCropPlan(request);
    const second = buildCropPlan(request);
    expect(first).toEqual(second);
    expect(first.crop.x).toBeGreaterThanOrEqual(0);
    expect(first.crop.y).toBeGreaterThanOrEqual(0);
    expect(first.crop.x + first.crop.width).toBeLessThanOrEqual(1);
    expect(first.crop.y + first.crop.height).toBeLessThanOrEqual(1);
    expect(first.bbox).toEqual(first.crop);
  });
});
