import { describe, expect, it } from "vitest";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import {
  loadProject,
  loadProjectDetails,
  loadProjectV2,
  migrateProjectToV2,
  projectPath,
  saveProject,
} from "../src/state.js";
import { initialTimeline } from "../src/state.js";

describe("versioned CLI project persistence", () => {
  it("loads v1, migrates it without mutating disk, and persists v2 explicitly", async () => {
    const dir = await mkdtemp(path.join(tmpdir(), "cut-state-v2-"));
    try {
      const legacy = initialTimeline("src1", "video.mp4", { duration: 4 });
      await saveProject(dir, legacy);
      expect((await loadProjectDetails(dir))?.version).toBe(1);
      expect((await loadProjectV2(dir))?.version).toBe(2);
      expect(JSON.parse(await readFile(projectPath(dir), "utf8"))).toMatchObject({ version: 1 });
      const migrated = await migrateProjectToV2(dir);
      expect(migrated?.version).toBe(2);
      expect(JSON.parse(await readFile(projectPath(dir), "utf8"))).toMatchObject({ version: 2 });
      expect((await loadProject(dir))?.version).toBe(2);
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });

  it("does not turn malformed JSON into a missing project", async () => {
    const dir = await mkdtemp(path.join(tmpdir(), "cut-state-v2-"));
    try {
      await writeFile(projectPath(dir), "{broken", "utf8");
      await expect(loadProject(dir)).rejects.toThrow(/malformed JSON/);
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });
});
