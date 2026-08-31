import { describe, expect, it } from "vitest";
import {
  RenderJobCancelledError,
  RenderJobManager,
} from "../src/render-job.js";

describe("RenderJobManager", () => {
  it("runs jobs with bounded concurrency and retains structured progress", async () => {
    const manager = new RenderJobManager({ concurrency: 1 });
    let running = 0;
    let peak = 0;
    const first = manager.start<number>(async ({ report }) => {
      running += 1;
      peak = Math.max(peak, running);
      report({ ratio: 0.4, phase: "render", current: 4, total: 10 });
      await new Promise((resolve) => setTimeout(resolve, 5));
      running -= 1;
      return 7;
    });
    const second = manager.start(async () => {
      running += 1;
      peak = Math.max(peak, running);
      running -= 1;
      return "done";
    });
    expect(await manager.result<number>(first)).toBe(7);
    expect(await manager.result<string>(second)).toBe("done");
    expect(peak).toBe(1);
    expect(manager.status(first)?.progress.ratio).toBe(1);
    expect(manager.status(first)?.progress.phase).toBe("complete");
  });

  it("cancels a queued callback before it starts", async () => {
    const manager = new RenderJobManager({ concurrency: 1 });
    let ran = false;
    const blocker = manager.start(async () => {
      await new Promise((resolve) => setTimeout(resolve, 15));
    });
    const cancelled = manager.start(async () => {
      ran = true;
    });
    expect(manager.cancel(cancelled)).toBe(true);
    await manager.result(blocker);
    await expect(manager.result(cancelled)).rejects.toBeInstanceOf(RenderJobCancelledError);
    expect(ran).toBe(false);
    expect(manager.status(cancelled)?.status).toBe("cancelled");
  });

  it("passes an AbortSignal to an injected callback", async () => {
    const manager = new RenderJobManager();
    let aborted = false;
    const id = manager.start(async ({ signal }) => {
      await new Promise<void>((resolve) => {
        signal.addEventListener("abort", () => {
          aborted = true;
          resolve();
        }, { once: true });
      });
      throw new RenderJobCancelledError();
    });
    expect(manager.cancel(id)).toBe(true);
    await expect(manager.result(id)).rejects.toBeInstanceOf(RenderJobCancelledError);
    expect(aborted).toBe(true);
  });

  it("supports a manager-level executor for v1/v2 hosts", async () => {
    const seen: string[] = [];
    const manager = new RenderJobManager({
      render: async (request, context) => {
        seen.push(request.kind ?? "generic");
        context.report(1);
        return request.kind;
      },
    });
    const id = manager.start({ kind: "host", metadata: { source: "test" } });
    await expect(manager.result(id)).resolves.toBe("host");
    expect(seen).toEqual(["host"]);
  });
});
