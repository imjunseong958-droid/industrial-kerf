import { describe, expect, it } from "vitest";
import { mkdtemp, mkdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import {
  loadSkills,
  parseSkillMd,
  skillsIndexPrompt,
  composeSystemPrompt,
  findSkill,
  createSkillReadTool,
  BUNDLED_SKILLS_DIR,
} from "../src/index.js";

async function makeSkillDir(root: string, name: string, fm: string, body = "body text\n"): Promise<string> {
  const dir = path.join(root, name);
  await mkdir(dir, { recursive: true });
  await writeFile(path.join(dir, "SKILL.md"), `---\n${fm}\n---\n${body}`);
  return dir;
}

describe("parseSkillMd", () => {
  it("parses frontmatter and body", () => {
    const parsed = parseSkillMd("---\nname: shorts\ndescription: Vertical recipes.\n---\n# Steps\n1. crop");
    expect(parsed).toEqual({
      name: "shorts",
      description: "Vertical recipes.",
      body: "# Steps\n1. crop",
    });
  });

  it("returns null without frontmatter or name", () => {
    expect(parseSkillMd("# just markdown")).toBeNull();
    expect(parseSkillMd("---\ndescription: no name\n---\nbody")).toBeNull();
  });
});

describe("loadSkills", () => {
  it("tolerates missing directories", async () => {
    const skills = await loadSkills([path.join(tmpdir(), "cut-nope-xyz")]);
    expect(skills).toEqual([]);
  });

  it("loads skills from a directory", async () => {
    const root = await mkdtemp(path.join(tmpdir(), "cut-skills-"));
    try {
      const dir = await makeSkillDir(root, "shorts", "name: shorts\ndescription: Vertical.");
      const skills = await loadSkills([root]);
      expect(skills).toEqual([{ name: "shorts", description: "Vertical.", body: "body text\n", dir }]);
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  });

  it("later dirs override earlier ones on name collision", async () => {
    const user = await mkdtemp(path.join(tmpdir(), "cut-user-"));
    const proj = await mkdtemp(path.join(tmpdir(), "cut-proj-"));
    try {
      await makeSkillDir(user, "shorts", "name: shorts\ndescription: user version");
      await makeSkillDir(user, "meme", "name: meme\ndescription: memes");
      await makeSkillDir(proj, "shorts", "name: shorts\ndescription: project version");
      const skills = await loadSkills([user, proj]);
      expect(skills.map((s) => s.name).sort()).toEqual(["meme", "shorts"]);
      const shorts = skills.find((s) => s.name === "shorts")!;
      expect(shorts.description).toBe("project version");
      expect(shorts.dir.startsWith(proj)).toBe(true);
    } finally {
      await rm(user, { recursive: true, force: true });
      await rm(proj, { recursive: true, force: true });
    }
  });

  it("finds bundled example skills in the repo", async () => {
    const skills = await loadSkills([BUNDLED_SKILLS_DIR]);
    expect(skills.map((s) => s.name).sort()).toEqual(["meme", "shorts", "stream-gameplay"]);
    for (const s of skills) {
      expect(s.description.length).toBeGreaterThan(0);
      expect(s.body).toContain("\n");
    }
  });

  it("stream-gameplay documents canonical vision classes", async () => {
    const skills = await loadSkills([BUNDLED_SKILLS_DIR]);
    const streamGameplay = skills.find((s) => s.name === "stream-gameplay")!;
    expect(streamGameplay.body).toContain("`cam`");
    expect(streamGameplay.body).toContain("`chat`");
    expect(streamGameplay.body).toContain("`content`");
    expect(streamGameplay.body).toMatch(/main gameplay\/stream canvas/i);
    expect(streamGameplay.body).toMatch(/main-picture ROI/i);
    expect(streamGameplay.body).not.toMatch(/\bwebcam\b/i);
  });
});

describe("findSkill and createSkillReadTool", () => {
  const skills = [
    { name: "shorts", description: "9:16 vertical recipes.", body: "full shorts body", dir: "/x/shorts" },
    { name: "meme", description: "memes", body: "full meme body", dir: "/x/meme" },
  ];

  it("matches exact names only", () => {
    expect(findSkill(skills, "shorts")).toEqual(skills[0]);
    expect(findSkill(skills, "Shorts")).toBeUndefined();
    expect(findSkill(skills, "shorts/SKILL.md")).toBeUndefined();
  });

  it("does not treat names as paths", () => {
    expect(findSkill(skills, "../meme")).toBeUndefined();
    expect(findSkill(skills, skills[0]!.dir)).toBeUndefined();
    expect(findSkill(skills, "/x/shorts/SKILL.md")).toBeUndefined();
  });

  it("returns the loaded playbook body by name", async () => {
    const tool = createSkillReadTool(skills);
    expect(tool.inputJsonSchema).toEqual({
      type: "object",
      properties: { name: { type: "string", minLength: 1 } },
      required: ["name"],
      additionalProperties: false,
    });
    await expect(tool.execute({ name: "shorts" })).resolves.toEqual({
      name: "shorts",
      description: "9:16 vertical recipes.",
      body: "full shorts body",
    });
  });

  it("throws a clear error for unknown names without other skill paths", async () => {
    const tool = createSkillReadTool(skills);
    await expect(tool.execute({ name: "missing" })).rejects.toThrow(/Unknown skill "missing"/);
    await expect(tool.execute({ name: "missing" })).rejects.toThrow(/Known skills: meme, shorts/);
    const traversal = await tool.execute({ name: "../meme" }).then(
      () => undefined,
      (error: unknown) => (error instanceof Error ? error.message : String(error)),
    );
    expect(traversal).toMatch(/Unknown skill "\.\.\/meme"/);
    expect(traversal).not.toMatch(/\/x\//);
    const missing = await tool.execute({ name: "missing" }).then(
      () => undefined,
      (error: unknown) => (error instanceof Error ? error.message : String(error)),
    );
    expect(missing).not.toMatch(/\/x\//);
  });

  it("lists no known skills when the catalog is empty", async () => {
    const tool = createSkillReadTool([]);
    await expect(tool.execute({ name: "shorts" })).rejects.toThrow(/No skills are loaded/);
  });
});

describe("system prompt composition", () => {
  const skills = [
    { name: "shorts", description: "9:16 vertical recipes.", body: "", dir: "/x/shorts" },
    { name: "meme", description: "", body: "", dir: "/x/meme" },
  ];

  it("builds a compact index pointing at skill_read", () => {
    const prompt = skillsIndexPrompt(skills);
    expect(prompt).toContain("- shorts: 9:16 vertical recipes.");
    expect(prompt).toContain("- meme: (no description)");
    expect(prompt).toContain('Call skill_read with { "name": "<skill-name>" }');
    expect(prompt).not.toContain("<dir>/SKILL.md");
    expect(prompt).not.toMatch(/\bbash\b/);
  });

  it("appends index to base prompt; empty skills returns base unchanged", () => {
    expect(composeSystemPrompt("base", [])).toBe("base");
    const composed = composeSystemPrompt("base", skills);
    expect(composed.startsWith("base\nAvailable skills:")).toBe(true);
  });
});
