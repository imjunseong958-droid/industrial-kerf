import { describe, expect, it } from "vitest";
import { getAgentConfig, importMedia, loadState, mediaUrl, normalizeRenderJob, normalizeState, outputUrl, resetAgentSession, sendCommand, stateFromResponse, updateAgentConfig } from "./api";

describe("GUI API client", () => {
  it("normalizes the host state envelope and falls back to project sources", () => {
    const state = normalizeState({
      project: {
        version: 2,
        name: "Interview",
        duration: 8,
        sources: [{ id: "camera one", path: "/media/camera.mp4" }],
        tracks: [],
      },
      revision: 4,
      canUndo: true,
      canRedo: false,
      duration: 8,
    });

    expect(state.revision).toBe(4);
    expect(state.sources).toEqual([{ id: "camera one", path: "/media/camera.mp4" }]);
    expect(stateFromResponse({ state })).toEqual(state);
  });

  it("builds encoded media and output endpoints", () => {
    expect(mediaUrl("camera one")).toBe("/api/media/camera%20one");
    expect(outputUrl("job/1")).toBe("/api/output/job%2F1");
  });

  it("rejects an invalid response instead of creating a fake project", () => {
    expect(stateFromResponse({ text: "nothing" })).toBeUndefined();
  });

  it("flattens the host's nested render response and marks it terminal", () => {
    const job = normalizeRenderJob("render-42", {
      jobId: "render-42",
      status: {
        status: "completed",
        state: "completed",
        output: "/tmp/render-42.mp4",
      },
      output: "/tmp/render-42.mp4",
    });

    expect(job.status).toBe("completed");
    expect(job.state).toBe("completed");
    expect(job.outputUrl).toBe("/api/output/render-42");
    expect(typeof job.status).toBe("string");
  });

  it("preserves a queued nested status and normalizes nested errors", () => {
    expect(normalizeRenderJob("render-43", { status: { status: "queued", state: "queued" } })).toMatchObject({
      jobId: "render-43",
      status: "queued",
      state: "queued",
    });
    expect(normalizeRenderJob("render-44", { status: { status: "failed", error: { message: "ffmpeg exited" } } })).toMatchObject({
      status: "failed",
      error: "ffmpeg exited",
    });
  });

  it("resets the agent session through the UI-only endpoint", async () => {
    const originalFetch = globalThis.fetch;
    let requestUrl = "";
    let requestInit: RequestInit | undefined;
    globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
      requestUrl = String(input);
      requestInit = init;
      return new Response(JSON.stringify({ ok: true }), {
        status: 200,
        headers: { "content-type": "application/json" },
      });
    }) as typeof fetch;
    try {
      await resetAgentSession();
    } finally {
      globalThis.fetch = originalFetch;
    }
    expect(requestUrl).toBe("/api/agent/reset");
    expect(requestInit?.method).toBe("POST");
  });

  it("normalizes and updates the optional agent model configuration", async () => {
    const originalFetch = globalThis.fetch;
    const requests: Array<{ url: string; init?: RequestInit }> = [];
    globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
      requests.push({ url: String(input), init });
      return new Response(JSON.stringify(requests.length === 1
        ? { provider: "openai", model: "gpt-5.4", options: { providers: ["anthropic", "openai"], models: { anthropic: ["claude-sonnet"], openai: ["gpt-5.4", "gpt-5.6"] } } }
        : { config: { provider: "openai", model: "gpt-5.6" } }), { status: 200 });
    }) as typeof fetch;
    try {
      await expect(getAgentConfig()).resolves.toMatchObject({
        provider: "openai",
        model: "gpt-5.4",
        providers: ["anthropic", "openai"],
        models: [
          { id: "claude-sonnet", provider: "anthropic" },
          { id: "gpt-5.4", provider: "openai" },
          { id: "gpt-5.6", provider: "openai" },
        ],
      });
      await updateAgentConfig({ model: "gpt-5.6", provider: "openai" });
    } finally {
      globalThis.fetch = originalFetch;
    }
    expect(requests[0]?.url).toBe("/api/agent/config");
    expect(requests[0]?.init?.method).toBe("GET");
    expect(requests[1]?.init?.method).toBe("POST");
    expect(JSON.parse(String(requests[1]?.init?.body))).toEqual({ model: "gpt-5.6", provider: "openai" });
  });

  it("sends YouTube imports as JSON and browser files as multipart form data", async () => {
    const originalFetch = globalThis.fetch;
    const requests: Array<{ url: string; init?: RequestInit }> = [];
    globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
      requests.push({ url: String(input), init });
      return new Response(JSON.stringify({ sourceId: "new-source" }), {
        status: 201,
        headers: { "content-type": "application/json" },
      });
    }) as typeof fetch;
    try {
      await importMedia({ url: " https://youtu.be/example " });
      await importMedia({ file: new File(["video"], "clip.mp4", { type: "video/mp4" }) });
    } finally {
      globalThis.fetch = originalFetch;
    }

    expect(requests[0]?.url).toBe("/api/import");
    expect(requests[0]?.init?.headers).toMatchObject({ "Content-Type": "application/json" });
    expect(JSON.parse(String(requests[0]?.init?.body))).toEqual({ url: "https://youtu.be/example" });
    expect(requests[1]?.init?.body).toBeInstanceOf(FormData);
    expect((requests[1]?.init?.headers as Record<string, string> | undefined)?.["Content-Type"]).toBeUndefined();
    expect(Array.from((requests[1]?.init?.body as FormData).keys())).toEqual(["file"]);
  });

  it("uses the typed Tauri command bus for desktop state and mutations", async () => {
    const previousWindow = (globalThis as { window?: unknown }).window;
    const calls: Array<{ command: string; args: unknown }> = [];
    Object.defineProperty(globalThis, "window", {
      configurable: true,
      value: {
        __TAURI_INTERNALS__: {
          invoke: async (command: string, args: unknown) => {
            calls.push({ command, args });
            return { project: { version: 2, fps: 30, sources: [], tracks: [], overlays: [], gaps: [], transitions: [], effects: [], markers: [] }, revision: 1, canUndo: true, canRedo: false, duration: 0, sources: [] };
          },
        },
      },
    });
    try {
      await loadState();
      await sendCommand({ type: "set_metadata", patch: { intent: "agentic" } }, 1);
      await importMedia("/tmp/example.mp4", 1, "media-1");
    } finally {
      if (previousWindow === undefined) delete (globalThis as { window?: unknown }).window;
      else Object.defineProperty(globalThis, "window", { configurable: true, value: previousWindow });
    }
    expect(calls.map((call) => call.command)).toEqual(["desktop_get_state", "desktop_apply_command", "desktop_import_media"]);
    expect(calls[1]?.args).toMatchObject({ request: { actor: "ui", baseRevision: 1, command: { type: "set_metadata" } } });
    expect((calls[1]?.args as { request: { idempotencyKey: string } }).request.idempotencyKey).toBeTruthy();
    expect(calls[2]?.args).toMatchObject({ request: { actor: "ui", baseRevision: 1, idempotencyKey: "media-1", path: "/tmp/example.mp4" } });
  });
});
