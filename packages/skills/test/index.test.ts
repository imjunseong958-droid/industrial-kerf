import { describe, expect, it } from "vitest";
import { mkdtemp, mkdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import {
  loadSkills,
  parseSkillMd,
  skillsIndexPrompt,
  composeSystemPrompt,
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
    expect(skills.map((s) => s.name).sort()).toEqual(["meme", "shorts"]);
    for (const s of skills) {
      expect(s.description.length).toBeGreaterThan(0);
      expect(s.body).toContain("\n");
    }
  });
});

describe("system prompt composition", () => {
  const skills = [
    { name: "shorts", description: "9:16 vertical recipes.", body: "", dir: "/x/shorts" },
    { name: "meme", description: "", body: "", dir: "/x/meme" },
  ];

  it("builds a compact index pointing at SKILL.md files", () => {
    const prompt = skillsIndexPrompt(skills);
    expect(prompt).toContain("- shorts — 9:16 vertical recipes.");
    expect(prompt).toContain("- meme — (no description)");
    expect(prompt).toContain("read the full skill file at <dir>/SKILL.md");
  });

  it("appends index to base prompt; empty skills returns base unchanged", () => {
    expect(composeSystemPrompt("base", [])).toBe("base");
    const composed = composeSystemPrompt("base", skills);
    expect(composed.startsWith("base\nAvailable skills:")).toBe(true);
  });
});
