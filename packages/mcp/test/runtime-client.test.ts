import { describe, expect, it, vi } from "vitest";
import { createCutMcpProjectHost } from "../src/host.js";
import { createCutMcpRouter } from "../src/server.js";
import type { RuntimeClient, RuntimeState } from "../src/runtime-client.js";

function state(): RuntimeState {
  return {
    project: { version: 2, fps: 30, sources: [], tracks: [], overlays: [], gaps: [], transitions: [], effects: [], markers: [] },
    revision: 0,
    canUndo: false,
    canRedo: false,
    duration: 0,
    sources: [],
  };
}

describe("Rust runtime client host delegation", () => {
  it("delegates MCP mutations and history to the shared runtime", async () => {
    let current = state();
    const runtime: RuntimeClient = {
      getState: vi.fn(async () => current),
      apply: vi.fn(async (_actor, baseRevision, _key, command) => {
        expect(baseRevision).toBe(0);
        expect(command).toMatchObject({ type: "update_clip" });
        current = { ...current, revision: 1, canUndo: true };
        return current;
      }),
      undo: vi.fn(async () => current),
      redo: vi.fn(async () => current),
      importMedia: vi.fn(async () => ({ state: current, sourceId: "source-1", clipId: "clip-1" })),
      close: vi.fn(),
    };
    const host = createCutMcpProjectHost({ dir: "/tmp/cut-runtime-client-test", runtime });
    const router = createCutMcpRouter(host.handlers);
    const result = await router.call("timeline.apply", { expectedRevision: 0, command: { type: "update_clip", clipId: "c1", patch: { opacity: 0.5 } } });
    expect(result.structuredContent).toMatchObject({ ok: true, result: { revision: 1 } });
    expect(runtime.apply).toHaveBeenCalledWith("mcp", 0, expect.any(String), expect.objectContaining({ type: "update_clip" }));
    await host.close();
  });
});
