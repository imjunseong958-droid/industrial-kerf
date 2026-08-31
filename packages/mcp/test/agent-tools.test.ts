import { describe, expect, it, afterEach, beforeEach, vi } from "vitest";
import { mkdtemp, mkdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { createCutAgentTools } from "../src/agent-tools.js";
import { createCutMcpProjectHost } from "../src/host.js";

describe("createCutAgentTools", () => {
  const dirs: string[] = [];
  beforeEach(() => {
    vi.stubEnv("HOME", tmpdir());
  });
  afterEach(async () => {
    await Promise.all(dirs.splice(0).map((dir) => rm(dir, { recursive: true, force: true })));
  });

  it("exposes core agent tools without edit or bash", async () => {
    const dir = await mkdtemp(path.join(tmpdir(), "cut-mcp-agent-tools-"));
    dirs.push(dir);
    const host = createCutMcpProjectHost({ dir });
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
    expect(tools.edit).toBeUndefined();
    expect(tools.bash).toBeUndefined();
    expect(tools.track_objects).toBeUndefined();
    expect(tools.timeline_query).toBeUndefined();
    expect(tools.timeline_apply).toBeUndefined();
    expect(tools.render_preview).toBeUndefined();
    expect(tools["skills.read"]).toBeUndefined();
    await host.close();
  });

  it("converts probe frames into multimodal_tool_result", async () => {
    const dir = await mkdtemp(path.join(tmpdir(), "cut-mcp-agent-probe-"));
    dirs.push(dir);
    const mediaPath = path.join(dir, "clip.mp4");
    const framePath = path.join(dir, ".cut", "frames", "frame_0.png");
    await writeFile(mediaPath, "video");
    await mkdir(path.dirname(framePath), { recursive: true });
    await writeFile(framePath, Buffer.from("png-bytes"));
    const host = createCutMcpProjectHost({
      dir,
      info: vi.fn().mockResolvedValue({ duration: 10, hasAudio: true }),
      extractFrames: vi.fn().mockResolvedValue([{ time: 2, path: framePath }]),
    });
    const tools = createCutAgentTools(host);
    const result = await tools.probe!.execute({
      action: "frames",
      src: "clip.mp4",
      opts: { frames: 1 },
    });
    expect(result).toMatchObject({
      type: "multimodal_tool_result",
      text: expect.stringContaining(framePath),
      metadata: [{ time: 2, path: framePath }],
      images: [{ data: expect.any(String), mediaType: "image/png" }],
    });
    await host.close();
  });

  it("includes track_objects only when vision is configured", async () => {
    const dir = await mkdtemp(path.join(tmpdir(), "cut-mcp-agent-vision-"));
    dirs.push(dir);
    const host = createCutMcpProjectHost({
      dir,
      vision: { track: vi.fn().mockResolvedValue({ detections: [] }) },
    });
    const tools = createCutAgentTools(host);
    expect(tools.track_objects).toBeDefined();
    await host.close();
  });

  it("registers skill_read against loaded skills without exposing skills.read", async () => {
    const dir = await mkdtemp(path.join(tmpdir(), "cut-mcp-agent-skill-"));
    dirs.push(dir);
    const host = createCutMcpProjectHost({ dir });
    const tools = createCutAgentTools(host, {
      skills: [{ name: "shorts", description: "vertical", body: "full body", dir: "/x/shorts" }],
    });
    expect(tools.skill_read).toBeDefined();
    expect(tools["skills.read"]).toBeUndefined();
    await expect(tools.skill_read!.execute({ name: "shorts" })).resolves.toEqual({
      name: "shorts",
      description: "vertical",
      body: "full body",
    });
    await host.close();
  });
});
