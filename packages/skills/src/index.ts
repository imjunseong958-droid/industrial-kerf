/**
 * @cut/skills. pi/Claude-compatible SKILL.md loader.
 *
 * A skill is a directory containing SKILL.md with YAML frontmatter:
 *
 *   ---
 *   name: shorts
 *   description: Recipe for 9:16 vertical shorts.
 *   ---
 *   Markdown instructions body…
 *
 * Skills are loaded as a compact index only; the agent reads the full
 * body on demand via skill_read. Project-level dirs override
 * user-level ones on name collision.
 */
import { readFile, readdir } from "node:fs/promises";
import { existsSync } from "node:fs";
import { homedir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

export interface Skill {
  name: string;
  description: string;
  /** Full markdown body below the frontmatter. */
  body: string;
  /** Directory containing this skill's SKILL.md. */
  dir: string;
}

/** Repo-bundled example skills (shipped alongside this package). */
export const BUNDLED_SKILLS_DIR = fileURLToPath(new URL("../../../skills", import.meta.url));

/** Default search order: project-level first (overrides user-level). */
export function defaultSkillDirs(cwd: string = process.cwd()): string[] {
  return [path.join(cwd, ".cut", "skills"), path.join(homedir(), ".cut", "skills")];
}

/**
 * Parse one SKILL.md into its frontmatter fields and body.
 * Returns null when frontmatter is missing or has no name.
 */
export function parseSkillMd(raw: string): { name: string; description: string; body: string } | null {
  const match = /^---\r?\n([\s\S]*?)\r?\n---\r?\n?([\s\S]*)$/.exec(raw);
  if (!match) return null;
  const fm = match[1] ?? "";
  const body = match[2] ?? "";
  const fields: Record<string, string> = {};
  for (const line of fm.split(/\r?\n/)) {
    const kv = /^([A-Za-z][\w-]*):\s*(.*)$/.exec(line.trim());
    if (kv) fields[kv[1]!.toLowerCase()] = kv[2]!.trim().replace(/^["']|["']$/g, "");
  }
  if (!fields["name"]) return null;
  return { name: fields["name"], description: fields["description"] ?? "", body };
}

async function loadSkillDir(dir: string): Promise<Skill | null> {
  const file = path.join(dir, "SKILL.md");
  let raw: string;
  try {
    raw = await readFile(file, "utf-8");
  } catch {
    return null; // missing dir or SKILL.md, tolerated
  }
  const parsed = parseSkillMd(raw);
  if (!parsed) return null;
  return { ...parsed, dir };
}

/**
 * Load skills from the given directories, in order. Later dirs win on
 * name collision (project overrides user when project comes last).
 */
export async function loadSkills(dirs: string[]): Promise<Skill[]> {
  const byName = new Map<string, Skill>();
  for (const root of dirs) {
    if (!existsSync(root)) continue;
    let entries: import("node:fs").Dirent[];
    try {
      entries = await readdir(root, { withFileTypes: true });
    } catch {
      continue;
    }
    for (const entry of entries) {
      if (!entry.isDirectory()) continue;
      const skill = await loadSkillDir(path.join(root, entry.name));
      if (skill) byName.set(skill.name, skill);
    }
  }
  return [...byName.values()];
}

/** Exact name match only. Never treats `name` as a filesystem path. */
export function findSkill(skills: readonly Skill[], name: string): Skill | undefined {
  return skills.find((skill) => skill.name === name);
}

function knownSkillNames(skills: readonly Skill[]): string {
  return [...new Set(skills.map((skill) => skill.name))].sort().join(", ");
}

function unknownSkillError(skills: readonly Skill[], name: string): Error {
  const known = knownSkillNames(skills);
  return new Error(
    known
      ? `Unknown skill ${JSON.stringify(name)}. Known skills: ${known}.`
      : `Unknown skill ${JSON.stringify(name)}. No skills are loaded.`,
  );
}

const SKILL_READ_INPUT_JSON_SCHEMA = {
  type: "object",
  properties: { name: { type: "string", minLength: 1 } },
  required: ["name"],
  additionalProperties: false,
} as const;

function skillReadName(input: unknown): string {
  if (typeof input !== "object" || input === null || Array.isArray(input)) {
    throw new Error('skill_read requires { "name": "<skill-name>" }');
  }
  const name = (input as { name?: unknown }).name;
  if (typeof name !== "string" || name.length === 0) {
    throw new Error("skill_read requires a non-empty name");
  }
  return name;
}

/** ToolImpl-shaped reader that returns a loaded playbook body by exact name. */
export function createSkillReadTool(skills: readonly Skill[]): {
  description: string;
  inputJsonSchema: {
    type: "object";
    properties: { name: { type: "string"; minLength: 1 } };
    required: ["name"];
    additionalProperties: false;
  };
  execute(input: unknown): Promise<{ name: string; description: string; body: string }>;
} {
  const loaded = [...skills];
  return {
    description: "Read a loaded playbook body by name.",
    inputJsonSchema: { ...SKILL_READ_INPUT_JSON_SCHEMA },
    async execute(input: unknown) {
      const name = skillReadName(input);
      const skill = findSkill(loaded, name);
      if (!skill) throw unknownSkillError(loaded, name);
      return { name: skill.name, description: skill.description, body: skill.body };
    },
  };
}

/**
 * Compact system-prompt index for loaded skills. Keeps context minimal.
 * The agent reads full bodies on demand via skill_read.
 */
export function skillsIndexPrompt(skills: Skill[]): string {
  if (skills.length === 0) return "";
  const lines = skills.map((s) => `- ${s.name}: ${s.description || "(no description)"}`);
  return [
    "Available skills:",
    ...lines,
    'Call skill_read with { "name": "<skill-name>" } to load a playbook body.',
  ].join("\n");
}

/** Compose a base system prompt with an optional skills index appended. */
export function composeSystemPrompt(base: string, skills: Skill[]): string {
  const index = skillsIndexPrompt(skills);
  return index ? `${base}\n${index}` : base;
}
