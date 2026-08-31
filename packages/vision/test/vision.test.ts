import { describe, expect, it, vi } from "vitest";
import {
  VisionAbortError,
  VisionConfigError,
  VisionOutputError,
  VisionProcessError,
  VisionTimeoutError,
  Yolo26Tracker,
  createYolo26Tracker,
  validateYolo26Config,
} from "../src/index.js";
import type { ProcessInvocation, ProcessResult } from "../src/index.js";

const CONFIG = {
  weights: "/tmp/custom-cut-yolo26s.pt",
  labelMap: {
    cam: "cam",
    chat: "chat",
    content: "content",
  },
} as const;

function ok(stdout: string): ProcessResult {
  return { stdout, stderr: "", exitCode: 0 };
}

describe("YOLO26 tracking config", () => {
  it("rejects the stock COCO small checkpoint and incomplete mappings", () => {
    expect(() => validateYolo26Config({ weights: "yolo26s.pt", labelMap: CONFIG.labelMap })).toThrow(
      VisionConfigError,
    );
    expect(() => validateYolo26Config({ weights: "yolo26s.onnx", labelMap: CONFIG.labelMap })).toThrow(
      VisionConfigError,
    );
    expect(() => validateYolo26Config({ weights: "/tmp/custom.pt", labelMap: { cam: "cam" } })).toThrow(
      /all three targets \(cam, chat, and content\)/,
    );
    expect(() =>
      validateYolo26Config({
        weights: "/tmp/custom.pt",
        labelMap: { webcam: "cam", chat: "chat", content: "content" },
      }),
    ).toThrow(/labelMap key "webcam" is not supported/);
  });

  it("accepts custom weights and the identity label map", () => {
    const omitted = validateYolo26Config({ weights: "./custom.pt" });
    expect(omitted.labelMap.get("cam")).toBe("cam");
    expect(omitted.labelMap.get("0")).toBe("cam");
    expect(omitted.labelMap.get("2")).toBe("content");

    const identity = validateYolo26Config({
      weights: "./custom.pt",
      labelMap: { cam: "cam", chat: "chat", content: "content" },
    });
    expect(identity.labelMap.get("cam")).toBe("cam");
    expect(identity.labelMap.get("chat")).toBe("chat");
    expect(identity.labelMap.get("content")).toBe("content");

    const config = validateYolo26Config({
      weights: " ./custom.pt ",
      labelMap: { " Cam ": "cam", chat: "chat", content: "content" },
    });
    expect(config.weights).toBe("./custom.pt");
    expect(config.labelMap.get("cam")).toBe("cam");
    expect(config.labelMap.get("chat")).toBe("chat");
    expect(config.labelMap.get("content")).toBe("content");
  });
});

describe("Yolo26Tracker", () => {
  it("accepts an injected runner for JSONL fixtures without a model file", async () => {
    const calls: ProcessInvocation[] = [];
    const runner = vi.fn(async (invocation: ProcessInvocation): Promise<ProcessResult> => {
      calls.push(invocation);
      return ok([
        JSON.stringify({
          frame_index: 1,
          time_seconds: 0.04,
          width: 1000,
          height: 500,
          detections: [
            { track_id: 11, class_name: "chat", confidence: 0.7, bbox: [500, 100, 900, 400] },
            { track_id: 2, class_name: "cam", confidence: 0.9, bbox: [100, 50, 300, 250] },
            { track_id: 99, class_name: "car", confidence: 0.99, bbox: [0, 0, 100, 100] },
          ],
        }),
      ].join("\n"));
    });
    const tracker = createYolo26Tracker(CONFIG, { runner });
    const result = await tracker.track({ source: "/tmp/input.mp4", fps: 25 });

    expect(calls).toHaveLength(1);
    expect(calls[0]?.command).toBe("python3");
    expect(calls[0]?.args).toContain("--model");
    expect(calls[0]?.args).toContain(CONFIG.weights);
    expect(calls[0]?.args).toContain("bytetrack.yaml");
    expect(calls[0]?.args).toContain("--fps");
    expect(result.frameCount).toBe(2);
    expect(result.detections).toEqual([
      {
        trackId: 2,
        class: "cam",
        confidence: 0.9,
        frame: 1,
        time: 0.04,
        frameIndex: 1,
        timeSeconds: 0.04,
        bbox: { x: 0.1, y: 0.1, width: 0.2, height: 0.4 },
      },
      {
        trackId: 11,
        class: "chat",
        confidence: 0.7,
        frame: 1,
        time: 0.04,
        frameIndex: 1,
        timeSeconds: 0.04,
        bbox: { x: 0.5, y: 0.2, width: 0.4, height: 0.6 },
      },
    ]);
  });

  it("supports normalized xywh boxes and can fail on unknown labels", async () => {
    const runner = async (): Promise<ProcessResult> =>
      ok(JSON.stringify({
        frame: 0,
        detections: [
          { track_id: "7", class_name: "cam", confidence: 0.5, bbox: { x: 0.2, y: 0.3, width: 0.4, height: 0.2 }, normalized: true },
          { track_id: 8, class_name: "other", confidence: 0.4, bbox: [0, 0, 1, 1], normalized: true },
        ],
      }));
    const tracker = new Yolo26Tracker({
      ...CONFIG,
      unknownLabelPolicy: "error",
    }, { runner });
    await expect(tracker.track("frame.jpg")).rejects.toBeInstanceOf(VisionOutputError);
  });

  it("normalizes absolute and normalized object xywh exactly once and preserves object xyxy", async () => {
    const runner = async (): Promise<ProcessResult> =>
      ok(JSON.stringify({
        frame: 0,
        width: 1000,
        height: 500,
        detections: [
          {
            track_id: 1,
            class_name: "cam",
            confidence: 0.9,
            bbox: { x: 100, y: 50, width: 200, height: 100 },
            bbox_format: "xywh",
          },
          {
            track_id: 2,
            class_name: "cam",
            confidence: 0.9,
            bbox: { x: 0.2, y: 0.3, width: 0.4, height: 0.2 },
            bbox_format: "xywh",
            normalized: true,
          },
          {
            track_id: 3,
            class_name: "cam",
            confidence: 0.9,
            bbox: { x1: 100, y1: 50, x2: 300, y2: 250 },
            bbox_format: "xyxy",
          },
        ],
      }));
    const tracker = createYolo26Tracker(CONFIG, { runner });
    const result = await tracker.track("frame.jpg");
    expect(result.detections.map((item) => item.bbox)).toEqual([
      { x: 0.1, y: 0.1, width: 0.2, height: 0.2 },
      { x: 0.2, y: 0.3, width: 0.4, height: 0.2 },
      { x: 0.1, y: 0.1, width: 0.2, height: 0.4 },
    ]);
  });

  it("maps non-zero process exits to a typed error", async () => {
    const tracker = createYolo26Tracker(CONFIG, {
      runner: async () => ({ stdout: "", stderr: "missing ultralytics", exitCode: 3 }),
    });
    await expect(tracker.track("input.mp4")).rejects.toMatchObject({
      constructor: VisionProcessError,
      exitCode: 3,
      stderr: "missing ultralytics",
    });
  });

  it("enforces timeout even when an injected runner ignores abort", async () => {
    const runner = async (): Promise<ProcessResult> => new Promise(() => undefined);
    const tracker = createYolo26Tracker({ ...CONFIG, timeoutMs: 10 }, { runner });
    await expect(tracker.track("input.mp4")).rejects.toBeInstanceOf(VisionTimeoutError);
  });

  it("propagates caller abort", async () => {
    const controller = new AbortController();
    const runner = async (): Promise<ProcessResult> => new Promise(() => undefined);
    const tracker = createYolo26Tracker(CONFIG, { runner });
    const pending = tracker.track("input.mp4", { signal: controller.signal, timeoutMs: 10_000 });
    controller.abort();
    await expect(pending).rejects.toBeInstanceOf(VisionAbortError);
  });
});
