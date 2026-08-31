#!/usr/bin/env -S npx tsx
/**
 * cut-mcp. Project-scoped MCP stdio host.
 *
 * The process writes only MCP protocol frames to stdout. Diagnostics belong on
 * stderr so it is safe to launch this command from an MCP client.
 */
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { pathToFileURL } from "node:url";
import path from "node:path";
import process from "node:process";
import { homedir } from "node:os";
import { BUNDLED_SKILLS_DIR, findSkill, loadSkills } from "@cut/skills";
import { createCutMcpServer, type CutMcpProfile } from "./server.js";
import { createCutMcpProjectHost } from "./host.js";

export const CUT_MCP_USAGE = "usage: cut-mcp [--dir <project-directory>] [--full]";

export interface CutMcpArgs {
  dir: string;
  profile: CutMcpProfile;
}

export function parseCutMcpArgs(
  argv: readonly string[],
  env: NodeJS.ProcessEnv = process.env,
): CutMcpArgs {
  let dir = env["CUT_PROJECT_DIR"] ?? process.cwd();
  let profile: CutMcpProfile = env["CUT_MCP_PROFILE"] === "full" ? "full" : "core";
  for (let index = 0; index < argv.length; index++) {
    const arg = argv[index];
    if (arg === undefined) continue;
    if (arg === "--dir" || arg === "-d") {
      const next = argv[index + 1];
      if (!next || next.startsWith("-")) throw new Error("cut-mcp --dir requires a path");
      dir = next;
      index++;
    } else if (arg === "--full") {
      profile = "full";
    } else if (arg === "--help" || arg === "-h") {
      throw new Error(CUT_MCP_USAGE);
    } else if (arg.startsWith("-")) {
      throw new Error(`unknown cut-mcp option: ${arg}`);
    }
  }
  return { dir: path.resolve(dir), profile };
}

export function parseCutMcpDir(argv: readonly string[], env: NodeJS.ProcessEnv = process.env): string {
  return parseCutMcpArgs(argv, env).dir;
}

async function readSkill(dir: string, input: { name: string }) {
  const skills = await loadSkills([
    BUNDLED_SKILLS_DIR,
    path.join(homedir(), ".cut", "skills"),
    path.join(dir, ".cut", "skills"),
  ]);
  const skill = findSkill(skills, input.name);
  if (!skill) {
    const known = skills.map((entry) => entry.name).sort().join(", ");
    throw new Error(
      known
        ? `Unknown skill ${JSON.stringify(input.name)}. Known skills: ${known}.`
        : `Unknown skill ${JSON.stringify(input.name)}. No skills are loaded.`,
    );
  }
  return { name: skill.name, description: skill.description, body: skill.body };
}

export async function startCutMcpStdio(
  argv: readonly string[] = process.argv.slice(2),
  env: NodeJS.ProcessEnv = process.env,
): Promise<{ close: () => Promise<void> }> {
  const { dir, profile } = parseCutMcpArgs(argv, env);
  const host = createCutMcpProjectHost({ dir });
  const server = createCutMcpServer(
    {
      ...host.handlers,
      skills: { read: (input) => readSkill(dir, input) },
    },
    {
      name: "cut-mcp",
      version: "0.1.0",
      profile,
      instructions:
        "Edits live in project.cut.json. " +
        "Read the timeline with timeline.query, then apply edits with timeline.apply using v2 commands or legacy op/ops. " +
        "Use media.probe with action=frames when you need visual context for crops. " +
        "Add sources with media.fetch or media.import. " +
        "Render with render.preview, render.start, and render.result. " +
        "Load playbooks with skills.read by skill name. Shell access is not available. " +
        "Default profile is core; pass --full or CUT_MCP_PROFILE=full for inspect/validate/diff/status/cancel/vision.",
    },
  );
  const transport = new StdioServerTransport();
  await server.connect(transport);
  return {
    close: async () => {
      await server.close();
      await host.close();
    },
  };
}

async function main(): Promise<void> {
  try {
    await startCutMcpStdio();
  } catch (error) {
    process.stderr.write(`cut-mcp: ${error instanceof Error ? error.message : String(error)}\n`);
    process.exitCode = 1;
  }
}

const invoked = process.argv[1];
if (invoked && import.meta.url === pathToFileURL(invoked).href) {
  void main();
}
