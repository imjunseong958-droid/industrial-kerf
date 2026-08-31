import { describe, expect, it, vi } from "vitest";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { buildCutTools } from "../src/tools.js";
import type { Yolo26Tracker } from "@cut/vision";

describe("optional track_objects tool", () => {
  it("is absent without trusted setup and exposes only safe input fields when configured", async () => {
    const dir = await mkdtemp(path.join(tmpdir(), "cut-vision-tools-"));
    vi.stubEnv("HOME", dir);
    try {
      expect("track_objects" in buildCutTools({ dir })).toBe(false);
      const tracker = {
        track: vi.fn(async ({ source }: { source: string }) => ({
          source,
          model: "/models/custom.pt",
          targets: ["cam", "chat", "content"] as const,
          frameCount: 1,
          detections: [
            {
              trackId: 1,
              class: "cam" as const,
              confidence: 0.9,
              frame: 0,
              frameIndex: 0,
              time: 0,
              timeSeconds: 0,
              bbox: { x: 0.1, y: 0.1, width: 0.2, height: 0.2 },
            },
            {
              trackId: 7,
              class: "content" as const,
              confidence: 0.9,
              frame: 0,
              frameIndex: 0,
              time: 0,
              timeSeconds: 0,
              bbox: { x: 0.2, y: 0.25, width: 0.45, height: 0.5 },
            },
          ],
        })),
      } as unknown as Yolo26Tracker;
      const tools = buildCutTools({ dir, visionTracker: tracker });
      expect("track_objects" in tools).toBe(true);
      const trackObjects = tools.track_objects;
      expect(trackObjects).toBeDefined();
      expect(trackObjects!.inputJsonSchema).not.toHaveProperty("properties.weights");
      const result = await trackObjects!.execute({
        source: "media/input.mp4",
        frameWidth: 1920,
        frameHeight: 1080,
        crop: { aspectRatio: 9 / 16 },
      });
      expect(result).toMatchObject({
        source: path.join(dir, "media/input.mp4"),
        targets: ["cam", "chat", "content"],
        plan: { byClass: { cam: expect.any(Array), chat: [], content: expect.any(Array) } },
        cropPlan: { crop: expect.any(Object) },
      });
      const crop = (result as { cropPlan: { crop: { width: number; height: number } } }).cropPlan.crop;
      expect((crop.width * 1920) / (crop.height * 1080)).toBeCloseTo(9 / 16, 5);
      expect(result).toMatchObject({
        cropPlan: {
          aspectRatio: 9 / 16,
          sourceAspectRatio: 16 / 9,
          normalizedAspectRatio: (9 / 16) / (16 / 9),
        },
      });
      expect(tracker.track).toHaveBeenCalledWith({
        source: path.join(dir, "media/input.mp4"),
        frameWidth: 1920,
        frameHeight: 1080,
      });
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });
});
