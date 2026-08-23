import { readFile } from "node:fs/promises";
import { homedir } from "node:os";
import path from "node:path";

/** One stdio MCP server entry from mcp.json. */
export interface McpServerConfig {
  command: string;
  args?: string[];
  env?: Record<string, string>;
}

/** Shape of ~/.cut/mcp.json (and ./.cut/mcp.json). */
export interface McpConfig {
  mcpServers: Record<string, McpServerConfig>;
}

/**
 * Load MCP server config by merging ~/.cut/mcp.json then <cwd>/.cut/mcp.json
 * (project entries win on name collisions). Missing files are skipped silently;
 * malformed files throw so callers can warn.
 */
export async function loadMcpConfig(opts?: {
  home?: string;
  cwd?: string;
  read?: (p: string) => Promise<string>;
}): Promise<McpConfig> {
  const read = opts?.read ?? ((p: string) => readFile(p, "utf-8"));
  const home = opts?.home ?? homedir();
  const cwd = opts?.cwd ?? process.cwd();
  const merged: McpConfig = { mcpServers: {} };
  for (const file of [
    path.join(home, ".cut", "mcp.json"),
    path.join(cwd, ".cut", "mcp.json"),
  ]) {
    let raw: string;
    try {
      raw = await read(file);
    } catch {
      continue;
    }
    const parsed = JSON.parse(raw) as Partial<McpConfig>;
    if (!parsed || typeof parsed !== "object" || !parsed.mcpServers) continue;
    for (const [name, cfg] of Object.entries(parsed.mcpServers)) {
      if (!cfg || typeof cfg.command !== "string") continue;
      merged.mcpServers[name] = cfg as McpServerConfig;
    }
  }
  return merged;
}
