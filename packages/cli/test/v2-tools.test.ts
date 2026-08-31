import { describe, expect, it } from "vitest";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { migrateV1ToV2 } from "@cut/timeline";
import { buildCutTools } from "../src/tools.js";
import { initialTimeline, saveProjectV2, projectPath } from "../src/state.js";

describe("CLI tools on v2 projects", () => {
  it("queries, edits, and renders v2 projects without using legacy edit", async () => {
    const dir = await mkdtemp(path.join(tmpdir(), "cut-v2-tools-"));
    try {
      const project = migrateV1ToV2(initialTimeline("src1", "/tmp/video.mp4", {
        duration: 10,
        width: 1920,
        height: 1080,
        hasAudio: true,
      }));
      await saveProjectV2(dir, project);
      const tools = buildCutTools({
        dir,
        render: {
          preview: async () => ({ jobId: "job-preview", output: path.join(dir, "preview.mp4"), revision: 0, status: { status: "queued" } }),
          start: async () => ({ jobId: "job-full", output: path.join(dir, "full.mp4"), revision: 0, status: { status: "queued" } }),
          status: async (input) => ({ jobId: input.jobId, status: { status: "completed" } }),
          cancel: async (input) => ({ jobId: input.jobId, cancelled: true, status: { status: "cancelled" } }),
          result: async (input) => ({
            jobId: input.jobId,
            status: { status: "completed" },
            output: input.jobId === "job-preview" ? path.join(dir, "preview.mp4") : path.join(dir, "full.mp4"),
          }),
        },
      });
      expect(tools.bash).toBeUndefined();
      expect(tools.edit).toBeUndefined();
      expect(tools.project_edit).toBeDefined();

      const timeline = await tools.timeline.execute({ include: "full" });
      expect(timeline.project.version).toBe(2);
      expect(timeline.tracks[0]!.clips[0]).toMatchObject({
        id: "c1",
        timelineStart: 0,
        duration: 10,
      });

      const edited = await tools.project_edit!.execute({
        expectedRevision: 0,
        command: { type: "update_clip", clipId: "c1", patch: { in: 2, out: 8 } },
      });
      expect(edited).toMatchObject({ revision: 1, project: { version: 2 } });
      expect(JSON.parse(await readFile(projectPath(dir), "utf8"))).toMatchObject({
        version: 2,
        tracks: [{ clips: [{ in: 2, out: 8 }] }],
      });

      await expect(tools.project_edit!.execute({
        expectedRevision: 0,
        command: { type: "update_clip", clipId: "c1", patch: { in: 3 } },
      })).rejects.toThrow(/revision conflict/);

      const rendered = await tools.render.execute({ draft: true });
      expect(rendered).toMatchObject({
        jobId: "job-preview",
        output: path.join(dir, "preview.mp4"),
      });
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });
});
