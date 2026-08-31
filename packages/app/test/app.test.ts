import { mkdtemp, readFile, readdir, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { createCutAgentTools, startCutApp } from "../src/index.js";
import { createCutMcpProjectHost } from "@cut/mcp";

const apps: Array<{ close: () => Promise<void> }> = [];
const dirs: string[] = [];

afterEach(async () => {
  for (const app of apps.splice(0)) await app.close();
  for (const dir of dirs.splice(0)) await rm(dir, { recursive: true, force: true });
});

async function tempDir(): Promise<string> {
  const dir = await mkdtemp(path.join(os.tmpdir(), "cut-app-"));
  dirs.push(dir);
  return dir;
}

describe("@cut/app", () => {
  it("serves canonical state and applies undoable commands", async () => {
    const app = await startCutApp({ dir: await tempDir() });
    apps.push(app);
    const initial = await (await fetch(app.url + "/api/state")).json() as {
      revision: number;
      project: { version: number };
      canUndo: boolean;
    };
    expect(initial.project.version).toBe(2);
    expect(initial.revision).toBe(0);
    expect(initial.canUndo).toBe(false);

    const applied = await (await fetch(app.url + "/api/command", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        expectedRevision: initial.revision,
        command: { type: "set_metadata", patch: { purpose: "test" } },
      }),
    })).json() as { revision: number; canUndo: boolean };
    expect(applied.revision).toBe(1);
    expect(applied.canUndo).toBe(true);

    const undone = await (await fetch(app.url + "/api/undo", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: "{}",
    })).json() as { revision: number; canRedo: boolean };
    expect(undone.revision).toBe(2);
    expect(undone.canRedo).toBe(true);
  });

  it("imports a probed local source atomically and supports range media reads", async () => {
    const dir = await tempDir();
    const media = path.join(dir, "sample.mp4");
    await writeFile(media, "sample-media");
    const app = await startCutApp({
      dir,
      probe: async () => ({ duration: 3, width: 640, height: 360, hasAudio: true }),
    });
    apps.push(app);

    const imported = await (await fetch(app.url + "/api/import", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ path: media }),
    })).json() as { sourceId: string; clipId: string; state: { sources: unknown[] } };
    expect(imported.sourceId).toBeTruthy();
    expect(imported.clipId).toBeTruthy();
    expect(imported.state.sources).toHaveLength(1);

    const response = await fetch(app.url + "/api/media/" + encodeURIComponent(imported.sourceId), {
      headers: { range: "bytes=0-5" },
    });
    expect(response.status).toBe(206);
    expect(await response.text()).toBe("sample");
    expect(await readFile(path.join(dir, "project.cut.json"), "utf8")).toContain(imported.sourceId);
  });

  it("imports a URL into project-owned storage and exposes fetched VTT as visual captions", async () => {
    const dir = await tempDir();
    let fetchedUrl = "";
    const app = await startCutApp({
      dir,
      probe: async () => ({ duration: 4, width: 1280, height: 720, hasAudio: true }),
      fetchVideo: async (url, outDir) => {
        fetchedUrl = url;
        const mediaPath = path.join(outDir, "video.mp4");
        const subsPath = path.join(outDir, "video.ko.vtt");
        await writeFile(mediaPath, "downloaded-media");
        await writeFile(subsPath, ["WEBVTT", "", "00:00.500 --> 00:02.000", "안녕하세요", ""].join("\n"));
        return { path: mediaPath, subsPath, title: "Fetched title" };
      },
    });
    apps.push(app);

    const response = await fetch(app.url + "/api/import", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ url: "https://www.youtube.com/watch?v=abc" }),
    });
    const imported = await response.json() as {
      source: { path: string; subs?: string; metadata?: { subtitleCueCount?: number } };
      subtitle?: { cueCount: number };
      state: { project: { overlays: Array<{ type?: string; cues?: Array<{ text: string; start: number }> }> } };
    };
    expect(response.status).toBe(201);
    expect(fetchedUrl).toBe("https://www.youtube.com/watch?v=abc");
    expect(imported.source.path).toContain(path.join(dir, "imports"));
    expect(imported.source.metadata?.subtitlePath).toContain(path.join(dir, "imports"));
    expect(imported.source.metadata?.subtitleCueCount).toBe(1);
    expect(imported.subtitle?.cueCount).toBe(1);
    expect(imported.state.project.overlays).toEqual(expect.arrayContaining([
      expect.objectContaining({
        type: "captions",
        cues: [expect.objectContaining({ text: "안녕하세요", start: 0.5 })],
      }),
    ]));
  });

  it("stores multipart browser uploads below imports and ignores unsafe filenames", async () => {
    const dir = await tempDir();
    const app = await startCutApp({
      dir,
      probe: async () => ({ duration: 2 }),
    });
    apps.push(app);
    const form = new FormData();
    form.append("file", new Blob(["uploaded-media"], { type: "video/mp4" }), "../../outside.mp4");
    const response = await fetch(app.url + "/api/import", { method: "POST", body: form });
    const imported = await response.json() as { source: { path: string } };
    expect(response.status).toBe(201);
    expect(imported.source.path).toContain(path.join(dir, "imports"));
    expect(path.basename(imported.source.path)).toBe("upload.mp4");
    expect(await readFile(imported.source.path, "utf8")).toBe("uploaded-media");
  });

  it("rolls back an uploaded artifact when probing fails", async () => {
    const dir = await tempDir();
    const app = await startCutApp({ dir, probe: async () => { throw new Error("probe failed"); } });
    apps.push(app);
    const form = new FormData();
    form.append("file", new Blob(["bad-media"], { type: "video/mp4" }), "sample.mp4");
    const response = await fetch(app.url + "/api/import", { method: "POST", body: form });
    expect(response.status).toBe(500);
    expect((await (await fetch(app.url + "/api/state")).json() as { revision: number }).revision).toBe(0);
    const imports = path.join(dir, "imports");
    expect(await readdir(imports)).toHaveLength(0);
  });

  it("exposes the shared MCP agent catalog without edit or bash", async () => {
    const dir = await tempDir();
    const host = createCutMcpProjectHost({ dir, vision: {} });
    const tools = createCutAgentTools(host);
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
    expect(Object.keys(tools).length).toBeLessThanOrEqual(8);
    expect(tools.skill_read).toBeDefined();
    expect(tools.edit).toBeUndefined();
    expect(tools.bash).toBeUndefined();
    await expect(tools.project_edit?.execute({
      command: {
        type: "replace_project",
        project: {
          version: 2,
          fps: 30,
          sources: [],
          tracks: [],
          overlays: [],
          gaps: [],
          transitions: [],
          effects: [],
          markers: [],
        },
      },
    })).rejects.toThrow(/replace_project/);
    await host.close();
  });

  it("does not accept arbitrary replace_project commands from HTTP", async () => {
    const app = await startCutApp({ dir: await tempDir() });
    apps.push(app);
    const response = await fetch(app.url + "/api/command", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        command: {
          type: "replace_project",
          project: { version: 2, fps: 30, sources: [], tracks: [], overlays: [], gaps: [], transitions: [], effects: [], markers: [] },
        },
      }),
    });
    expect(response.status).toBe(400);
  });

  it("keeps a persistent injected agent on the same project host", async () => {
    let calls = 0;
    const app = await startCutApp({
      dir: await tempDir(),
      agentSessionFactory: async (options) => {
        const listeners = new Set<(event: { type: string }) => void>();
        return {
          onEvent(listener: (event: { type: string }) => void) {
            listeners.add(listener);
            return () => listeners.delete(listener);
          },
          abort() {},
          async run() {
            calls += 1;
            for (const listener of listeners) listener({ type: "turn_started" });
            await options.tools?.project_edit?.execute({
              command: { type: "set_metadata", patch: { viaAgent: true } },
            });
            return "updated";
          },
        } as never;
      },
    });
    apps.push(app);
    const response = await fetch(app.url + "/api/agent", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ prompt: "update the project" }),
    });
    const result = await response.json() as {
      text: string;
      events: Array<{ type: string }>;
      state: { project: { metadata?: { viaAgent?: boolean } } };
    };
    expect(response.status).toBe(200);
    expect(calls).toBe(1);
    expect(result.text).toBe("updated");
    expect(result.events.map((event) => event.type)).toContain("turn_started");
    expect(result.state.project.metadata?.viaAgent).toBe(true);
  });

  it("clears only the agent session history and preserves project state", async () => {
    const messages = [{ role: "user", content: "remember this context" }];
    const session = {
      messages,
      onEvent() {
        return () => undefined;
      },
      abort() {},
      async run() {
        return "ok";
      },
    };
    const app = await startCutApp({ dir: await tempDir(), agentSession: session as never });
    apps.push(app);

    const before = await (await fetch(app.url + "/api/state")).json() as {
      revision: number;
      project: unknown;
    };
    const response = await fetch(app.url + "/api/agent/reset", { method: "POST" });

    expect(response.status).toBe(200);
    expect(await response.json()).toEqual({ ok: true });
    expect(session.messages).toEqual([]);

    const after = await (await fetch(app.url + "/api/state")).json() as {
      revision: number;
      project: unknown;
    };
    expect(after.revision).toBe(before.revision);
    expect(after.project).toEqual(before.project);
  });

  it("serializes a reset behind an in-flight agent turn", async () => {
    let releaseRun!: () => void;
    const runGate = new Promise<void>((resolve) => { releaseRun = resolve; });
    let runStarted = false;
    let runFinished = false;
    let resetDuringRun = false;
    let resetAfterRun = false;
    let history: unknown[] = [{ role: "user", content: "old context" }];
    const session = {
      get messages() {
        return history;
      },
      set messages(next: unknown[]) {
        if (runFinished) resetAfterRun = true;
        else resetDuringRun = true;
        history = next;
      },
      onEvent() {
        return () => undefined;
      },
      abort() {},
      async run() {
        runStarted = true;
        await runGate;
        runFinished = true;
        return "done";
      },
    };
    const app = await startCutApp({ dir: await tempDir(), agentSession: session as never });
    apps.push(app);

    const agentRequest = fetch(app.url + "/api/agent", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ prompt: "make an edit" }),
    });
    // Let the queued run start before asking for a reset.
    for (let attempt = 0; attempt < 100 && !runStarted; attempt += 1) {
      await new Promise((resolve) => setTimeout(resolve, 1));
    }
    expect(runStarted).toBe(true);
    const resetRequest = fetch(app.url + "/api/agent/reset", { method: "POST" });
    await new Promise((resolve) => setTimeout(resolve, 5));
    expect(resetDuringRun).toBe(false);

    releaseRun();
    const [agentResponse, resetResponse] = await Promise.all([agentRequest, resetRequest]);
    expect(agentResponse.status).toBe(200);
    expect(resetResponse.status).toBe(200);
    expect(resetAfterRun).toBe(true);
    expect(history).toEqual([]);
  });

  it("exposes safe agent config and applies changes to the next fresh session", async () => {
    const created: Array<Record<string, unknown>> = [];
    const app = await startCutApp({
      dir: await tempDir(),
      llm: {
        provider: "anthropic",
        model: "claude-custom",
        apiKey: "do-not-return",
        baseUrl: "https://secret.example/v1",
      },
      agentSessionFactory: async (options) => {
        created.push(options as unknown as Record<string, unknown>);
        return {
          messages: [],
          onEvent() {
            return () => undefined;
          },
          abort() {},
          async run() {
            return "configured";
          },
        } as never;
      },
    });
    apps.push(app);

    const initialResponse = await fetch(app.url + "/api/agent/config");
    expect(initialResponse.status).toBe(200);
    const initial = await initialResponse.json() as Record<string, unknown> & {
      options: { providers: string[]; models: Record<string, string[]> };
    };
    expect(initial.provider).toBe("anthropic");
    expect(initial.model).toBe("claude-custom");
    expect(initial.options.providers).toEqual(["anthropic", "openai"]);
    expect(initial.options.models.anthropic).toContain("claude-custom");
    expect(JSON.stringify(initial)).not.toContain("do-not-return");
    expect(JSON.stringify(initial)).not.toContain("secret.example");

    const changedResponse = await fetch(app.url + "/api/agent/config", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        provider: "openai",
        apiMode: "responses",
        model: "gpt-5.6-luna",
        reasoningEffort: "low",
      }),
    });
    expect(changedResponse.status).toBe(200);
    const changed = await changedResponse.json() as {
      ok: boolean;
      config: Record<string, unknown>;
    };
    expect(changed.ok).toBe(true);
    expect(changed.config).toMatchObject({
      provider: "openai",
      apiMode: "responses",
      model: "gpt-5.6-luna",
      reasoningEffort: "low",
    });

    const agentResponse = await fetch(app.url + "/api/agent", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ prompt: "use the selected model" }),
    });
    expect(agentResponse.status).toBe(200);
    expect(created).toHaveLength(1);
    expect(created[0]).toMatchObject({
      provider: "openai",
      apiMode: "responses",
      model: "gpt-5.6-luna",
      reasoningEffort: "low",
    });
  });

  it("rejects config updates that would replace an injected session without a factory", async () => {
    const session = {
      messages: [],
      provider: "anthropic",
      model: "claude-sonnet-4-20250514",
      onEvent() {
        return () => undefined;
      },
      abort() {},
      async run() {
        return "ok";
      },
    };
    const app = await startCutApp({ dir: await tempDir(), agentSession: session as never });
    apps.push(app);

    const response = await fetch(app.url + "/api/agent/config", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ provider: "openai" }),
    });
    expect(response.status).toBe(409);
    expect((await response.json() as { error: string }).error).toMatch(/cannot be replaced/);
  });
});
